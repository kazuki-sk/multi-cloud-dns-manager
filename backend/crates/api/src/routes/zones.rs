use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use dns_manager_db::DbPool;

// ── response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Zone {
    id: String,
    name: String,
    default_ttl: i64,
    owner_team_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ListZonesResponse {
    zones: Vec<Zone>,
}

// ── request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateZoneRequest {
    name: String,
    #[serde(default = "default_ttl")]
    default_ttl: i64,
}

fn default_ttl() -> i64 {
    300
}

// ── handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/zones — return all zones ordered by name.
pub async fn list_zones(
    State(pool): State<Arc<DbPool>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, name, default_ttl, owner_team_id, created_at, updated_at
         FROM zones
         ORDER BY name"
    )
    .fetch_all(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    let zones: Vec<Zone> = rows
        .into_iter()
        .map(|r| Zone {
            id: r.id,
            name: r.name,
            default_ttl: r.default_ttl,
            owner_team_id: r.owner_team_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(Json(ListZonesResponse { zones }))
}

/// POST /api/v1/zones — create a new zone.
///
/// Returns 201 Created with the zone JSON, or 409 Conflict if the name is
/// already in use, or 422 Unprocessable Entity on validation failure.
pub async fn create_zone(
    State(pool): State<Arc<DbPool>>,
    Json(body): Json<CreateZoneRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // ── validation ────────────────────────────────────────────────────────────
    if body.name.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "name must not be empty".into(),
        ));
    }
    if body.name.len() > 255 {
        return Err(ApiError::UnprocessableEntity(
            "name must be 255 characters or fewer".into(),
        ));
    }

    // ── insert ────────────────────────────────────────────────────────────────
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO zones (id, name, default_ttl, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)",
        id,
        body.name,
        body.default_ttl,
        now,
        now,
    )
    .execute(&*pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.is_unique_violation() {
                return ApiError::Conflict;
            }
        }
        ApiError::from(dns_manager_db::Error::Database(e))
    })?;

    // ── fetch created row ─────────────────────────────────────────────────────
    let row = sqlx::query!(
        "SELECT id, name, default_ttl, owner_team_id, created_at, updated_at
         FROM zones
         WHERE id = ?",
        id,
    )
    .fetch_one(&*pool)
    .await
    .map_err(|e| ApiError::from(dns_manager_db::Error::Database(e)))?;

    Ok((
        StatusCode::CREATED,
        Json(Zone {
            id: row.id,
            name: row.name,
            default_ttl: row.default_ttl,
            owner_team_id: row.owner_team_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }),
    ))
}

// ── stub handlers (not yet implemented) ──────────────────────────────────────

pub async fn get_zone() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn update_zone() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn delete_zone() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn list_records() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn create_record() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn update_record() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn delete_record() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
