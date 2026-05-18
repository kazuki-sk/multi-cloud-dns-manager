use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use dns_manager_core::ChangeSetStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use dns_manager_db::DbPool;

// ── status helpers ────────────────────────────────────────────────────────────

fn status_to_db(s: ChangeSetStatus) -> &'static str {
    match s {
        ChangeSetStatus::Draft => "draft",
        ChangeSetStatus::Validated => "validated",
        ChangeSetStatus::Applying => "applying",
        ChangeSetStatus::Applied => "applied",
        ChangeSetStatus::RollingBack => "rolling_back",
        ChangeSetStatus::RolledBack => "rolled_back",
        ChangeSetStatus::RollbackFailed => "rollback_failed",
        ChangeSetStatus::Frozen => "frozen",
    }
}

fn status_from_db(s: &str) -> Result<ChangeSetStatus, ApiError> {
    match s {
        "draft" => Ok(ChangeSetStatus::Draft),
        "validated" => Ok(ChangeSetStatus::Validated),
        "applying" => Ok(ChangeSetStatus::Applying),
        "applied" => Ok(ChangeSetStatus::Applied),
        "rolling_back" => Ok(ChangeSetStatus::RollingBack),
        "rolled_back" => Ok(ChangeSetStatus::RolledBack),
        "rollback_failed" => Ok(ChangeSetStatus::RollbackFailed),
        "frozen" => Ok(ChangeSetStatus::Frozen),
        other => {
            tracing::error!(status = other, "unknown changeset status in database");
            Err(ApiError::Internal(format!("unknown changeset status: {other}")))
        }
    }
}

// ── DB row types ──────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct ChangesetRow {
    id: String,
    created_by: String,
    description: Option<String>,
    status: String,
    rollback_policy: String,
    created_at: String,
    updated_at: String,
}

#[derive(FromRow)]
struct ChangesetItemRow {
    record_id: String,
    operation: String,
    before_value: Option<String>,
    after_value: Option<String>,
}

// ── response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChangesetItem {
    record_id: String,
    operation: String,
    before_value: Option<JsonValue>,
    after_value: Option<JsonValue>,
}

#[derive(Serialize)]
struct Changeset {
    id: String,
    created_by: String,
    description: Option<String>,
    status: String,
    rollback_policy: String,
    created_at: String,
    updated_at: String,
    items: Vec<ChangesetItem>,
}

// ── request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateChangesetItemRequest {
    record_id: String,
    operation: String,
    before_value: Option<JsonValue>,
    after_value: Option<JsonValue>,
}

#[derive(Deserialize)]
pub struct CreateChangesetRequest {
    description: Option<String>,
    #[serde(default = "default_rollback_policy")]
    rollback_policy: String,
    items: Vec<CreateChangesetItemRequest>,
}

fn default_rollback_policy() -> String {
    "auto".to_string()
}

// ── shared helpers ────────────────────────────────────────────────────────────

fn db_err(e: sqlx::Error) -> ApiError {
    ApiError::from(dns_manager_db::Error::Database(e))
}

/// Fetch a changeset row and its items by ID. Returns `None` if not found.
async fn fetch_changeset(
    pool: &DbPool,
    changeset_id: &str,
) -> Result<Option<Changeset>, ApiError> {
    let Some(row) = sqlx::query_as::<_, ChangesetRow>(
        "SELECT id, created_by, description, status, rollback_policy, created_at, updated_at
         FROM changesets WHERE id = ?",
    )
    .bind(changeset_id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    else {
        return Ok(None);
    };

    let item_rows = sqlx::query_as::<_, ChangesetItemRow>(
        "SELECT record_id, operation, before_value, after_value
         FROM changeset_items WHERE changeset_id = ?
         ORDER BY record_id",
    )
    .bind(changeset_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;

    let items = item_rows
        .into_iter()
        .map(|r| {
            let before_value = r
                .before_value
                .as_deref()
                .map(serde_json::from_str::<JsonValue>)
                .transpose()
                .map_err(|e| ApiError::Internal(format!("item before_value parse error: {e}")))?;
            let after_value = r
                .after_value
                .as_deref()
                .map(serde_json::from_str::<JsonValue>)
                .transpose()
                .map_err(|e| ApiError::Internal(format!("item after_value parse error: {e}")))?;
            Ok(ChangesetItem {
                record_id: r.record_id,
                operation: r.operation,
                before_value,
                after_value,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Some(Changeset {
        id: row.id,
        created_by: row.created_by,
        description: row.description,
        status: row.status,
        rollback_policy: row.rollback_policy,
        created_at: row.created_at,
        updated_at: row.updated_at,
        items,
    }))
}

// ── handlers ──────────────────────────────────────────────────────────────────

/// POST /api/v1/changesets — create a changeset with items in a single transaction.
pub async fn create_changeset(
    State(pool): State<Arc<DbPool>>,
    Json(body): Json<CreateChangesetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // ── validation ────────────────────────────────────────────────────────────
    if body.items.is_empty() {
        return Err(ApiError::BadRequest("items must not be empty".into()));
    }
    let valid_ops = ["create", "update", "delete"];
    for item in &body.items {
        if !valid_ops.contains(&item.operation.as_str()) {
            return Err(ApiError::UnprocessableEntity(format!(
                "invalid operation '{}': must be one of create, update, delete",
                item.operation
            )));
        }
        if item.operation == "create" && item.after_value.is_none() {
            return Err(ApiError::UnprocessableEntity(
                "after_value is required for 'create' operation".into(),
            ));
        }
        if item.operation == "delete" && item.before_value.is_none() {
            return Err(ApiError::UnprocessableEntity(
                "before_value is required for 'delete' operation".into(),
            ));
        }
    }
    let valid_policies = ["auto", "manual", "frozen_on_failure"];
    if !valid_policies.contains(&body.rollback_policy.as_str()) {
        return Err(ApiError::UnprocessableEntity(format!(
            "invalid rollback_policy '{}': must be one of auto, manual, frozen_on_failure",
            body.rollback_policy
        )));
    }

    // ── transaction ───────────────────────────────────────────────────────────
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let mut tx = pool.begin().await.map_err(db_err)?;

    sqlx::query(
        "INSERT INTO changesets (id, created_by, description, status, rollback_policy, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind("anonymous")
    .bind(&body.description)
    .bind(status_to_db(ChangeSetStatus::Draft))
    .bind(&body.rollback_policy)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    for item in &body.items {
        let before_str = item
            .before_value
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| ApiError::Internal(format!("before_value serialization failed: {e}")))?;
        let after_str = item
            .after_value
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| ApiError::Internal(format!("after_value serialization failed: {e}")))?;

        sqlx::query(
            "INSERT INTO changeset_items (changeset_id, record_id, operation, before_value, after_value)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&item.record_id)
        .bind(&item.operation)
        .bind(&before_str)
        .bind(&after_str)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }

    tx.commit().await.map_err(db_err)?;

    // ── fetch created changeset ───────────────────────────────────────────────
    let changeset = fetch_changeset(&pool, &id)
        .await?
        .ok_or_else(|| ApiError::Internal("changeset disappeared after insert".into()))?;

    Ok((StatusCode::CREATED, Json(changeset)))
}

/// GET /api/v1/changesets/{changeset_id}
pub async fn get_changeset(
    State(pool): State<Arc<DbPool>>,
    Path(changeset_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let changeset = fetch_changeset(&pool, &changeset_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(changeset))
}

/// POST /api/v1/changesets/{changeset_id}/validate — transition draft → validated.
pub async fn validate_changeset(
    State(pool): State<Arc<DbPool>>,
    Path(changeset_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    transition_changeset(&pool, &changeset_id, ChangeSetStatus::Validated).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tower::ServiceExt;

    async fn make_pool() -> Arc<DbPool> {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        dns_manager_db::migrate(&pool).await.unwrap();
        Arc::new(pool)
    }

    fn make_router(pool: Arc<DbPool>) -> Router {
        Router::new()
            .route("/changesets", post(create_changeset))
            .route("/changesets/{id}/validate", post(validate_changeset))
            .with_state(pool)
    }

    fn json_body(v: serde_json::Value) -> Body {
        Body::from(serde_json::to_vec(&v).unwrap())
    }

    #[tokio::test]
    async fn empty_items_returns_400() {
        let pool = make_pool().await;
        let app = make_router(pool);
        let req = Request::builder()
            .method("POST")
            .uri("/changesets")
            .header("content-type", "application/json")
            .body(json_body(serde_json::json!({ "items": [] })))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invalid_state_transition_returns_400() {
        let pool = make_pool().await;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        // Insert a changeset already in 'applying' state.
        // validate_changeset attempts applying → validated, which is forbidden.
        sqlx::query(
            "INSERT INTO changesets
             (id, created_by, description, status, rollback_policy, created_at, updated_at)
             VALUES (?, 'test', NULL, 'applying', 'auto', ?, ?)",
        )
        .bind(&id)
        .bind(&now)
        .bind(&now)
        .execute(&*pool)
        .await
        .unwrap();

        let app = make_router(Arc::clone(&pool));
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/changesets/{id}/validate"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

#[derive(Serialize)]
struct ListChangesetsResponse {
    changesets: Vec<Changeset>,
}

/// GET /api/v1/changesets — list all changesets, newest first.
pub async fn list_changesets(
    State(pool): State<Arc<DbPool>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = sqlx::query_as::<_, ChangesetRow>(
        "SELECT id, created_by, description, status, rollback_policy, created_at, updated_at
         FROM changesets ORDER BY created_at DESC",
    )
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    #[derive(sqlx::FromRow)]
    struct ItemWithCsId {
        changeset_id: String,
        record_id: String,
        operation: String,
        before_value: Option<String>,
        after_value: Option<String>,
    }

    let all_items: Vec<ItemWithCsId> = sqlx::query_as(
        "SELECT changeset_id, record_id, operation, before_value, after_value
         FROM changeset_items ORDER BY changeset_id, record_id",
    )
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    let mut items_map: std::collections::HashMap<String, Vec<ChangesetItem>> =
        std::collections::HashMap::new();
    for r in all_items {
        let before_value = r
            .before_value
            .as_deref()
            .map(serde_json::from_str::<JsonValue>)
            .transpose()
            .map_err(|e| ApiError::Internal(format!("item before_value parse error: {e}")))?;
        let after_value = r
            .after_value
            .as_deref()
            .map(serde_json::from_str::<JsonValue>)
            .transpose()
            .map_err(|e| ApiError::Internal(format!("item after_value parse error: {e}")))?;
        items_map
            .entry(r.changeset_id)
            .or_default()
            .push(ChangesetItem {
                record_id: r.record_id,
                operation: r.operation,
                before_value,
                after_value,
            });
    }

    let changesets = rows
        .into_iter()
        .map(|row| {
            let items = items_map.remove(&row.id).unwrap_or_default();
            Changeset {
                id: row.id,
                created_by: row.created_by,
                description: row.description,
                status: row.status,
                rollback_policy: row.rollback_policy,
                created_at: row.created_at,
                updated_at: row.updated_at,
                items,
            }
        })
        .collect();

    Ok(Json(ListChangesetsResponse { changesets }))
}

/// POST /api/v1/changesets/{id}/apply — transition validated → applying.
///
/// The worker picks up `applying` changesets and performs the DNS changes.
pub async fn apply_changeset(
    State(pool): State<Arc<DbPool>>,
    Path(changeset_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let target = ChangeSetStatus::Applying;
    transition_changeset(&pool, &changeset_id, target).await
}

/// POST /api/v1/changesets/{id}/rollback — transition applied → rolling_back.
///
/// The worker picks up `rolling_back` changesets and reverts DNS changes.
pub async fn rollback_changeset(
    State(pool): State<Arc<DbPool>>,
    Path(changeset_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let target = ChangeSetStatus::RollingBack;
    transition_changeset(&pool, &changeset_id, target).await
}

/// Common helper: read current status, validate the transition, apply it.
async fn transition_changeset(
    pool: &DbPool,
    changeset_id: &str,
    target: ChangeSetStatus,
) -> Result<impl IntoResponse, ApiError> {
    let mut tx = pool.begin().await.map_err(db_err)?;

    let row = sqlx::query_as::<_, ChangesetRow>(
        "SELECT id, created_by, description, status, rollback_policy, created_at, updated_at
         FROM changesets WHERE id = ?",
    )
    .bind(changeset_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_err)?
    .ok_or(ApiError::NotFound)?;

    let current = status_from_db(&row.status)?;

    if !current.can_transition_to(target) {
        return Err(ApiError::BadRequest(format!(
            "cannot transition from '{}' to '{}'",
            status_to_db(current),
            status_to_db(target),
        )));
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE changesets SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status_to_db(target))
        .bind(&now)
        .bind(changeset_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    tx.commit().await.map_err(db_err)?;

    let changeset = fetch_changeset(pool, changeset_id)
        .await?
        .ok_or_else(|| ApiError::Internal("changeset disappeared after update".into()))?;

    Ok(Json(changeset))
}
