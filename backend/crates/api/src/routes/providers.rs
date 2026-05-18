use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use dns_manager_core::{encrypt, KeyProvider};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use std::sync::Arc;
use uuid::Uuid;

const VALID_PROVIDER_TYPES: &[&str] = &["route53", "azuredns", "gcloud"];

use crate::error::ApiError;
use dns_manager_db::DbPool;

// ── DB row types ──────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct ProviderRow {
    id: String,
    name: String,
    provider_type: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(FromRow)]
struct ProviderZoneBindingRow {
    id: String,
    zone_id: String,
    zone_name: String,
    provider_zone_id: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(FromRow)]
struct SyncStateRow {
    record_id: String,
    provider_binding_id: String,
    last_observed_value: Option<String>,
    last_observed_at: Option<String>,
    status: String,
    last_error: Option<String>,
    retry_count: i64,
    created_at: String,
    updated_at: String,
}

// ── response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Provider {
    id: String,
    name: String,
    provider_type: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ListProvidersResponse {
    providers: Vec<Provider>,
}

#[derive(Serialize)]
struct ProviderZoneBinding {
    id: String,
    zone_id: String,
    zone_name: String,
    provider_zone_id: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ListProviderBindingsResponse {
    bindings: Vec<ProviderZoneBinding>,
}

#[derive(Serialize)]
struct SyncState {
    record_id: String,
    provider_binding_id: String,
    last_observed_value: Option<String>,
    last_observed_at: Option<String>,
    status: String,
    last_error: Option<String>,
    retry_count: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct SyncStateListResponse {
    sync_states: Vec<SyncState>,
}

// ── request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateProviderRequest {
    name: String,
    provider_type: String,
    credentials: JsonValue,
}

#[derive(Deserialize)]
pub struct UpdateProviderRequest {
    name: Option<String>,
    status: Option<String>,
}

// ── handlers ──────────────────────────────────────────────────────────────────

/// POST /api/v1/providers — create a cloud provider account with envelope-encrypted credentials.
///
/// Returns 201 with the created provider; credentials fields are never included in the response.
pub async fn create_provider(
    State(pool): State<Arc<DbPool>>,
    State(key_provider): State<Arc<dyn KeyProvider>>,
    Json(body): Json<CreateProviderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // ── validation ────────────────────────────────────────────────────────────
    if body.name.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "name must not be empty".into(),
        ));
    }
    if !VALID_PROVIDER_TYPES.contains(&body.provider_type.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "unknown provider_type '{}': must be one of {:?}",
            body.provider_type, VALID_PROVIDER_TYPES,
        )));
    }

    // ── encrypt credentials (envelope encryption) ─────────────────────────────
    let creds_bytes = serde_json::to_vec(&body.credentials)
        .map_err(|e| ApiError::Internal(format!("credentials serialization failed: {e}")))?;
    let encrypted = encrypt(&*key_provider, &creds_bytes).map_err(ApiError::from)?;

    // ── insert ────────────────────────────────────────────────────────────────
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO providers
             (id, name, provider_type, credentials_blob, credentials_dek, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.provider_type)
    .bind(&encrypted.blob)
    .bind(&encrypted.dek)
    .bind(&now)
    .bind(&now)
    .execute(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    // ── fetch created row (credentials columns excluded) ──────────────────────
    let row = sqlx::query_as::<_, ProviderRow>(
        "SELECT id, name, provider_type, status, created_at, updated_at
         FROM providers
         WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    Ok((
        StatusCode::CREATED,
        Json(Provider {
            id: row.id,
            name: row.name,
            provider_type: row.provider_type,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }),
    ))
}

/// GET /api/v1/providers — list all providers (credentials excluded).
pub async fn list_providers(
    State(pool): State<Arc<DbPool>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = sqlx::query_as::<_, ProviderRow>(
        "SELECT id, name, provider_type, status, created_at, updated_at
         FROM providers
         ORDER BY created_at",
    )
    .fetch_all(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    let providers = rows
        .into_iter()
        .map(|r| Provider {
            id: r.id,
            name: r.name,
            provider_type: r.provider_type,
            status: r.status,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(Json(ListProvidersResponse { providers }))
}

/// GET /api/v1/providers/{provider_id}/bindings — list zone bindings for a provider.
///
/// Returns 404 if the provider_id does not exist.
pub async fn list_provider_bindings(
    State(pool): State<Arc<DbPool>>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers WHERE id = ?")
        .bind(&provider_id)
        .fetch_one(&*pool)
        .await
        .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    if count == 0 {
        return Err(ApiError::NotFound);
    }

    let rows = sqlx::query_as::<_, ProviderZoneBindingRow>(
        "SELECT pb.id, pb.zone_id, z.name AS zone_name, pb.provider_zone_id,
                pb.status, pb.created_at, pb.updated_at
         FROM provider_bindings pb
         JOIN zones z ON pb.zone_id = z.id
         WHERE pb.provider_id = ?
         ORDER BY pb.created_at",
    )
    .bind(&provider_id)
    .fetch_all(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    let bindings = rows
        .into_iter()
        .map(|r| ProviderZoneBinding {
            id: r.id,
            zone_id: r.zone_id,
            zone_name: r.zone_name,
            provider_zone_id: r.provider_zone_id,
            status: r.status,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(Json(ListProviderBindingsResponse { bindings }))
}

/// GET /api/v1/providers/{provider_id}/sync-state — list sync states for a provider binding.
///
/// Returns 404 if the provider_id does not exist.
pub async fn get_sync_state(
    State(pool): State<Arc<DbPool>>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // ── provider existence check ──────────────────────────────────────────────
    let binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_bindings WHERE id = ?")
            .bind(&provider_id)
            .fetch_one(&*pool)
            .await
            .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    if binding_count == 0 {
        return Err(ApiError::NotFound);
    }

    // ── fetch sync states ─────────────────────────────────────────────────────
    let rows = sqlx::query_as::<_, SyncStateRow>(
        "SELECT record_id, provider_binding_id, last_observed_value, last_observed_at,
                status, last_error, retry_count, created_at, updated_at
         FROM sync_states
         WHERE provider_binding_id = ?
         ORDER BY record_id",
    )
    .bind(&provider_id)
    .fetch_all(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    let sync_states: Vec<SyncState> = rows
        .into_iter()
        .map(|r| SyncState {
            record_id: r.record_id,
            provider_binding_id: r.provider_binding_id,
            last_observed_value: r.last_observed_value,
            last_observed_at: r.last_observed_at,
            status: r.status,
            last_error: r.last_error,
            retry_count: r.retry_count,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(Json(SyncStateListResponse { sync_states }))
}

/// GET /api/v1/providers/{provider_id} — fetch a single provider.
pub async fn get_provider(
    State(pool): State<Arc<DbPool>>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let row = sqlx::query_as::<_, ProviderRow>(
        "SELECT id, name, provider_type, status, created_at, updated_at
         FROM providers WHERE id = ?",
    )
    .bind(&provider_id)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(Provider {
        id: row.id,
        name: row.name,
        provider_type: row.provider_type,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// PATCH /api/v1/providers/{provider_id} — update name and/or status.
pub async fn update_provider(
    State(pool): State<Arc<DbPool>>,
    Path(provider_id): Path<String>,
    Json(body): Json<UpdateProviderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.name.is_none() && body.status.is_none() {
        return Err(ApiError::UnprocessableEntity(
            "at least one of name or status must be provided".into(),
        ));
    }
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(ApiError::UnprocessableEntity(
                "name must not be empty".into(),
            ));
        }
    }
    const VALID_STATUSES: &[&str] = &["active", "paused", "error"];
    if let Some(ref status) = body.status {
        if !VALID_STATUSES.contains(&status.as_str()) {
            return Err(ApiError::BadRequest(format!(
                "unknown status '{}': must be one of {:?}",
                status, VALID_STATUSES,
            )));
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE providers
         SET name       = COALESCE(?, name),
             status     = COALESCE(?, status),
             updated_at = ?
         WHERE id = ?",
    )
    .bind(body.name.as_deref())
    .bind(body.status.as_deref())
    .bind(&now)
    .bind(&provider_id)
    .execute(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    let row = sqlx::query_as::<_, ProviderRow>(
        "SELECT id, name, provider_type, status, created_at, updated_at
         FROM providers WHERE id = ?",
    )
    .bind(&provider_id)
    .fetch_one(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    Ok(Json(Provider {
        id: row.id,
        name: row.name,
        provider_type: row.provider_type,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// DELETE /api/v1/providers/{provider_id} — delete a provider.
///
/// Returns 409 if zone bindings still reference this provider.
pub async fn delete_provider(
    State(pool): State<Arc<DbPool>>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers WHERE id = ?")
        .bind(&provider_id)
        .fetch_one(&*pool)
        .await
        .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;
    if count == 0 {
        return Err(ApiError::NotFound);
    }

    let binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_bindings WHERE provider_id = ?")
            .bind(&provider_id)
            .fetch_one(&*pool)
            .await
            .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;
    if binding_count > 0 {
        return Err(ApiError::Conflict);
    }

    sqlx::query("DELETE FROM providers WHERE id = ?")
        .bind(&provider_id)
        .execute(&*pool)
        .await
        .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::FromRef,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use dns_manager_core::EnvKeyProvider;
    use sqlx::sqlite::SqliteConnectOptions;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct TestState {
        pool: Arc<DbPool>,
        key_provider: Arc<dyn KeyProvider>,
    }

    impl FromRef<TestState> for Arc<DbPool> {
        fn from_ref(s: &TestState) -> Self {
            Arc::clone(&s.pool)
        }
    }

    impl FromRef<TestState> for Arc<dyn KeyProvider> {
        fn from_ref(s: &TestState) -> Self {
            Arc::clone(&s.key_provider)
        }
    }

    async fn make_state() -> TestState {
        use sqlx::sqlite::SqlitePoolOptions;
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        dns_manager_db::migrate(&pool).await.unwrap();
        TestState {
            pool: Arc::new(pool),
            key_provider: Arc::new(EnvKeyProvider),
        }
    }

    fn make_router(state: TestState) -> Router {
        Router::new()
            .route("/providers", post(create_provider))
            .with_state(state)
    }

    fn json_body(v: serde_json::Value) -> Body {
        Body::from(serde_json::to_vec(&v).unwrap())
    }

    #[tokio::test]
    async fn unknown_provider_type_returns_400() {
        let app = make_router(make_state().await);
        let req = Request::builder()
            .method("POST")
            .uri("/providers")
            .header("content-type", "application/json")
            .body(json_body(serde_json::json!({
                "name": "my-provider",
                "provider_type": "unknown",
                "credentials": {}
            })))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
