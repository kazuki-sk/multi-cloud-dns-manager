use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use dns_manager_core::RecordType;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::ApiError;
use dns_manager_db::DbPool;

// ── helpers ───────────────────────────────────────────────────────────────────

fn db_err(e: sqlx::Error) -> ApiError {
    ApiError::from(dns_manager_db::Error::Database(e))
}

/// Map a RecordType enum to its canonical DB / display string.
fn record_type_str(rt: RecordType) -> &'static str {
    match rt {
        RecordType::A => "A",
        RecordType::Aaaa => "AAAA",
        RecordType::Cname => "CNAME",
        RecordType::Mx => "MX",
        RecordType::Txt => "TXT",
        RecordType::Ns => "NS",
        RecordType::Srv => "SRV",
        RecordType::Caa => "CAA",
    }
}

/// SHA-256 hex of `name|record_type|ttl|sorted_values_json`.
/// Values are sorted before hashing so the hash is order-independent.
fn compute_desired_hash(name: &str, record_type: &str, ttl: i64, values: &[String]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(b"|");
    hasher.update(record_type.as_bytes());
    hasher.update(b"|");
    hasher.update(ttl.to_string().as_bytes());
    hasher.update(b"|");
    for value in sorted {
        hasher.update(value.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(value.as_bytes());
        hasher.update(b"|");
    }
    let bytes = hasher.finalize();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── response / DB row types ───────────────────────────────────────────────────

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

#[derive(FromRow)]
struct ZoneRow {
    id: String,
    name: String,
    default_ttl: i64,
    owner_team_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct Record {
    id: String,
    zone_id: String,
    name: String,
    record_type: String,
    ttl: i64,
    values: Vec<String>,
    desired_hash: String,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ListRecordsResponse {
    records: Vec<Record>,
}

#[derive(FromRow)]
struct RecordRow {
    id: String,
    zone_id: String,
    name: String,
    record_type: String,
    ttl: i64,
    record_values: String,
    desired_hash: String,
    created_at: String,
    updated_at: String,
}

impl RecordRow {
    fn into_record(self) -> Result<Record, ApiError> {
        let values = serde_json::from_str::<Vec<String>>(&self.record_values)
            .map_err(|e| ApiError::Internal(format!("record_values parse error: {e}")))?;
        Ok(Record {
            id: self.id,
            zone_id: self.zone_id,
            name: self.name,
            record_type: self.record_type,
            ttl: self.ttl,
            values,
            desired_hash: self.desired_hash,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

// ── request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateZoneRequest {
    name: String,
    #[serde(default = "default_zone_ttl")]
    default_ttl: i64,
}

fn default_zone_ttl() -> i64 {
    300
}

#[derive(Deserialize)]
pub struct CreateRecordRequest {
    name: String,
    record_type: RecordType,
    ttl: i64,
    values: Vec<String>,
}

// ── zone handlers ─────────────────────────────────────────────────────────────

/// GET /api/v1/zones — return all zones ordered by name.
pub async fn list_zones(State(pool): State<Arc<DbPool>>) -> Result<impl IntoResponse, ApiError> {
    let rows = sqlx::query_as::<_, ZoneRow>(
        "SELECT id, name, default_ttl, owner_team_id, created_at, updated_at
         FROM zones
         ORDER BY name",
    )
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    let zones = rows
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
pub async fn create_zone(
    State(pool): State<Arc<DbPool>>,
    Json(body): Json<CreateZoneRequest>,
) -> Result<impl IntoResponse, ApiError> {
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

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO zones (id, name, default_ttl, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(body.default_ttl)
    .bind(&now)
    .bind(&now)
    .execute(&*pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.is_unique_violation() {
                return ApiError::Conflict;
            }
        }
        db_err(e)
    })?;

    let row = sqlx::query_as::<_, ZoneRow>(
        "SELECT id, name, default_ttl, owner_team_id, created_at, updated_at
         FROM zones WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;

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

// ── record handlers ───────────────────────────────────────────────────────────

/// GET /api/v1/zones/{zone_id}/records — list active records for a zone.
pub async fn list_records(
    State(pool): State<Arc<DbPool>>,
    Path(zone_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let zone_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM zones WHERE id = ?")
        .bind(&zone_id)
        .fetch_one(&*pool)
        .await
        .map_err(db_err)?;
    if zone_count == 0 {
        return Err(ApiError::NotFound);
    }

    let rows = sqlx::query_as::<_, RecordRow>(
        "SELECT id, zone_id, name, record_type, ttl, record_values, desired_hash, created_at, updated_at
         FROM desired_records
         WHERE zone_id = ? AND deleted_at IS NULL
         ORDER BY name, record_type",
    )
    .bind(&zone_id)
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    let records = rows
        .into_iter()
        .map(RecordRow::into_record)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(ListRecordsResponse { records }))
}

/// POST /api/v1/zones/{zone_id}/records — create a DNS record in a zone.
///
/// Returns 404 if the zone does not exist, 409 on duplicate (zone, name, type),
/// 400 if ttl ≤ 0.
pub async fn create_record(
    State(pool): State<Arc<DbPool>>,
    Path(zone_id): Path<String>,
    Json(body): Json<CreateRecordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // ── validation ────────────────────────────────────────────────────────────
    if body.ttl <= 0 {
        return Err(ApiError::BadRequest("ttl must be greater than 0".into()));
    }
    if body.name.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "name must not be empty".into(),
        ));
    }
    if body.values.is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "values must not be empty".into(),
        ));
    }

    // ── zone existence check ──────────────────────────────────────────────────
    let zone_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM zones WHERE id = ?")
        .bind(&zone_id)
        .fetch_one(&*pool)
        .await
        .map_err(db_err)?;
    if zone_count == 0 {
        return Err(ApiError::NotFound);
    }

    // ── prepare values ────────────────────────────────────────────────────────
    let rt_str = record_type_str(body.record_type);
    let record_values_json = serde_json::to_string(&body.values)
        .map_err(|e| ApiError::Internal(format!("values serialization failed: {e}")))?;
    let desired_hash = compute_desired_hash(&body.name, rt_str, body.ttl, &body.values);
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    // ── insert ────────────────────────────────────────────────────────────────
    sqlx::query(
        "INSERT INTO desired_records
             (id, zone_id, name, record_type, record_values, ttl, desired_hash, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&zone_id)
    .bind(&body.name)
    .bind(rt_str)
    .bind(&record_values_json)
    .bind(body.ttl)
    .bind(&desired_hash)
    .bind(&now)
    .bind(&now)
    .execute(&*pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.is_unique_violation() {
                return ApiError::Conflict;
            }
        }
        db_err(e)
    })?;

    // ── fetch created row ─────────────────────────────────────────────────────
    let row = sqlx::query_as::<_, RecordRow>(
        "SELECT id, zone_id, name, record_type, ttl, record_values, desired_hash, created_at, updated_at
         FROM desired_records WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;

    Ok((StatusCode::CREATED, Json(row.into_record()?)))
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

pub async fn update_record() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

pub async fn delete_record() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
