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

use crate::error::ApiError;
use dns_manager_db::DbPool;

// ── DB row types ──────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct ProviderBindingRow {
    id: String,
    zone_id: String,
    provider_type: String,
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
struct ProviderBinding {
    id: String,
    zone_id: String,
    provider_type: String,
    provider_zone_id: String,
    status: String,
    created_at: String,
    updated_at: String,
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
    zone_id: String,
    provider_type: String,
    provider_zone_id: String,
    credentials: JsonValue,
}

// ── handlers ──────────────────────────────────────────────────────────────────

/// POST /api/v1/providers — create a provider binding with envelope-encrypted credentials.
///
/// Returns 404 if zone_id does not exist.
/// Returns 201 with the created binding; credentials fields are never included in the response.
pub async fn create_provider(
    State(pool): State<Arc<DbPool>>,
    State(key_provider): State<Arc<dyn KeyProvider>>,
    Json(body): Json<CreateProviderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // ── validation ────────────────────────────────────────────────────────────
    if body.provider_type.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "provider_type must not be empty".into(),
        ));
    }
    if body.provider_zone_id.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "provider_zone_id must not be empty".into(),
        ));
    }

    // ── zone existence check ──────────────────────────────────────────────────
    let zone_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM zones WHERE id = ?")
        .bind(&body.zone_id)
        .fetch_one(&*pool)
        .await
        .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    if zone_count == 0 {
        return Err(ApiError::NotFound);
    }

    // ── encrypt credentials (envelope encryption) ─────────────────────────────
    let creds_bytes = serde_json::to_vec(&body.credentials)
        .map_err(|e| ApiError::Internal(format!("credentials serialization failed: {e}")))?;
    let encrypted = encrypt(&*key_provider, &creds_bytes).map_err(ApiError::from)?;

    // ── insert ────────────────────────────────────────────────────────────────
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO provider_bindings
             (id, zone_id, provider_type, provider_zone_id, credentials_blob, credentials_dek,
              created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&body.zone_id)
    .bind(&body.provider_type)
    .bind(&body.provider_zone_id)
    .bind(&encrypted.blob)
    .bind(&encrypted.dek)
    .bind(&now)
    .bind(&now)
    .execute(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    // ── fetch created row (credentials columns excluded) ──────────────────────
    let row = sqlx::query_as::<_, ProviderBindingRow>(
        "SELECT id, zone_id, provider_type, provider_zone_id, status, created_at, updated_at
         FROM provider_bindings
         WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    Ok((
        StatusCode::CREATED,
        Json(ProviderBinding {
            id: row.id,
            zone_id: row.zone_id,
            provider_type: row.provider_type,
            provider_zone_id: row.provider_zone_id,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }),
    ))
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

// ── stub handlers (not yet implemented) ──────────────────────────────────────

pub async fn list_providers() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn get_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn update_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn delete_provider() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
