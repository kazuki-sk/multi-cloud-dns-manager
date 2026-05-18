use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use dns_manager_core::{ProviderRecord, RecordType};
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
    status: String,
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
    status: String,
    deleted_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl RecordRow {
    fn into_record(self) -> Result<Record, ApiError> {
        let values = serde_json::from_str::<Vec<String>>(&self.record_values)
            .map_err(|e| ApiError::Internal(format!("record_values parse error: {e}")))?;
        // Tombstoned records that haven't been pushed to DNS yet show as "pending_delete".
        let status = if self.deleted_at.is_some() {
            "pending_delete".to_string()
        } else {
            self.status
        };
        Ok(Record {
            id: self.id,
            zone_id: self.zone_id,
            name: self.name,
            record_type: self.record_type,
            ttl: self.ttl,
            values,
            desired_hash: self.desired_hash,
            status,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn parse_record_type(s: &str) -> Result<RecordType, ApiError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| ApiError::Internal(format!("record_type parse error '{}': {e}", s)))
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
pub struct UpdateZoneRequest {
    name: Option<String>,
    default_ttl: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateRecordRequest {
    name: String,
    record_type: RecordType,
    ttl: i64,
    values: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdateRecordRequest {
    ttl: Option<i64>,
    values: Option<Vec<String>>,
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
        "SELECT id, zone_id, name, record_type, ttl, record_values, desired_hash, status,
                deleted_at, created_at, updated_at
         FROM desired_records
         WHERE zone_id = ?
           AND (deleted_at IS NULL OR (deleted_at IS NOT NULL AND status = 'pending'))
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
             (id, zone_id, name, record_type, record_values, ttl, desired_hash, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)",
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
        "SELECT id, zone_id, name, record_type, ttl, record_values, desired_hash, status,
                deleted_at, created_at, updated_at
         FROM desired_records WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;

    Ok((StatusCode::CREATED, Json(row.into_record()?)))
}

// ── bindings row / response types ────────────────────────────────────────────

#[derive(FromRow)]
struct BindingRow {
    id: String,
    zone_id: String,
    provider_id: String,
    provider_zone_id: String,
    status: String,
    created_at: String,
    updated_at: String,
    provider_name: String,
    provider_type: String,
}

#[derive(Serialize)]
struct Binding {
    id: String,
    zone_id: String,
    provider_id: String,
    provider_name: String,
    provider_type: String,
    provider_zone_id: String,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ListBindingsResponse {
    bindings: Vec<Binding>,
}

#[derive(Deserialize)]
pub struct CreateBindingRequest {
    provider_id: String,
    provider_zone_id: String,
}

// ── binding handlers ──────────────────────────────────────────────────────────

/// GET /api/v1/zones/{zone_id}/bindings — list bindings for a zone.
pub async fn list_bindings(
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

    let rows = sqlx::query_as::<_, BindingRow>(
        "SELECT pb.id, pb.zone_id, pb.provider_id, pb.provider_zone_id,
                pb.status, pb.created_at, pb.updated_at,
                p.name AS provider_name, p.provider_type
         FROM provider_bindings pb
         JOIN providers p ON pb.provider_id = p.id
         WHERE pb.zone_id = ?
         ORDER BY pb.created_at",
    )
    .bind(&zone_id)
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    let bindings = rows
        .into_iter()
        .map(|r| Binding {
            id: r.id,
            zone_id: r.zone_id,
            provider_id: r.provider_id,
            provider_name: r.provider_name,
            provider_type: r.provider_type,
            provider_zone_id: r.provider_zone_id,
            status: r.status,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(Json(ListBindingsResponse { bindings }))
}

/// POST /api/v1/zones/{zone_id}/bindings — bind a provider to a zone.
///
/// Returns 404 if zone or provider does not exist.
pub async fn create_binding(
    State(pool): State<Arc<DbPool>>,
    Path(zone_id): Path<String>,
    Json(body): Json<CreateBindingRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.provider_id.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "provider_id must not be empty".into(),
        ));
    }
    if body.provider_zone_id.trim().is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "provider_zone_id must not be empty".into(),
        ));
    }

    let zone_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM zones WHERE id = ?")
        .bind(&zone_id)
        .fetch_one(&*pool)
        .await
        .map_err(db_err)?;
    if zone_count == 0 {
        return Err(ApiError::NotFound);
    }

    let provider_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers WHERE id = ?")
        .bind(&body.provider_id)
        .fetch_one(&*pool)
        .await
        .map_err(db_err)?;
    if provider_count == 0 {
        return Err(ApiError::NotFound);
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO provider_bindings
             (id, zone_id, provider_id, provider_zone_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&zone_id)
    .bind(&body.provider_id)
    .bind(&body.provider_zone_id)
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

    let row = sqlx::query_as::<_, BindingRow>(
        "SELECT pb.id, pb.zone_id, pb.provider_id, pb.provider_zone_id,
                pb.status, pb.created_at, pb.updated_at,
                p.name AS provider_name, p.provider_type
         FROM provider_bindings pb
         JOIN providers p ON pb.provider_id = p.id
         WHERE pb.id = ?",
    )
    .bind(&id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;

    Ok((
        StatusCode::CREATED,
        Json(Binding {
            id: row.id,
            zone_id: row.zone_id,
            provider_id: row.provider_id,
            provider_name: row.provider_name,
            provider_type: row.provider_type,
            provider_zone_id: row.provider_zone_id,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }),
    ))
}

// ── push pending records ──────────────────────────────────────────────────────

#[derive(FromRow)]
struct PendingRecordRow {
    id: String,
    name: String,
    record_type: String,
    ttl: i64,
    record_values: String,
}

#[derive(serde::Serialize)]
struct PushPendingResponse {
    changeset_id: String,
    record_count: usize,
}

/// POST /api/v1/zones/{zone_id}/push
///
/// Collects every `status = 'pending'` record for the zone, bundles them into a
/// new changeset that is created directly in `validated` state (ready for the
/// worker to pick up), and marks those records as `synced`.
///
/// Returns 422 if there are no pending records.
pub async fn push_pending_records(
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

    // Pending creates / updates (never tombstoned).
    let pending_creates: Vec<PendingRecordRow> = sqlx::query_as(
        "SELECT id, name, record_type, ttl, record_values
         FROM desired_records
         WHERE zone_id = ? AND status = 'pending' AND deleted_at IS NULL
         ORDER BY name, record_type",
    )
    .bind(&zone_id)
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    // Pending deletes (tombstoned but not yet submitted to DNS).
    let pending_deletes: Vec<PendingRecordRow> = sqlx::query_as(
        "SELECT id, name, record_type, ttl, record_values
         FROM desired_records
         WHERE zone_id = ? AND status = 'pending' AND deleted_at IS NOT NULL
         ORDER BY name, record_type",
    )
    .bind(&zone_id)
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    if pending_creates.is_empty() && pending_deletes.is_empty() {
        return Err(ApiError::UnprocessableEntity(
            "no pending records to push".into(),
        ));
    }

    let record_count = pending_creates.len() + pending_deletes.len();
    let changeset_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let mut tx = pool.begin().await.map_err(db_err)?;

    // Create changeset directly in 'validated' state so the worker picks it up.
    sqlx::query(
        "INSERT INTO changesets
             (id, created_by, description, status, rollback_policy, created_at, updated_at)
         VALUES (?, ?, NULL, 'validated', 'auto', ?, ?)",
    )
    .bind(&changeset_id)
    .bind("anonymous")
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    // ── create / update items ────────────────────────────────────────────────
    for row in &pending_creates {
        let values: Vec<String> = serde_json::from_str(&row.record_values)
            .map_err(|e| ApiError::Internal(format!("record_values parse error: {e}")))?;
        let ttl = u32::try_from(row.ttl)
            .map_err(|e| ApiError::Internal(format!("ttl conversion error: {e}")))?;
        let after_record = ProviderRecord {
            name: row.name.clone(),
            record_type: parse_record_type(&row.record_type)?,
            ttl,
            values,
        };
        let after_value_json = serde_json::to_string(&after_record)
            .map_err(|e| ApiError::Internal(format!("after_value serialization failed: {e}")))?;

        let sync_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sync_states WHERE record_id = ?")
                .bind(&row.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?;

        let (operation, before_value): (&str, Option<String>) = if sync_count > 0 {
            let before: Option<String> = sqlx::query_scalar(
                "SELECT last_observed_value FROM sync_states
                 WHERE record_id = ? AND last_observed_value IS NOT NULL
                 ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(&row.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?
            .flatten();
            ("update", before)
        } else {
            ("create", None)
        };

        sqlx::query(
            "INSERT INTO changeset_items
                 (changeset_id, record_id, operation, before_value, after_value)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&changeset_id)
        .bind(&row.id)
        .bind(operation)
        .bind(&before_value)
        .bind(&after_value_json)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }

    // ── delete items ─────────────────────────────────────────────────────────
    for row in &pending_deletes {
        let values: Vec<String> = serde_json::from_str(&row.record_values)
            .map_err(|e| ApiError::Internal(format!("record_values parse error: {e}")))?;
        let ttl = u32::try_from(row.ttl)
            .map_err(|e| ApiError::Internal(format!("ttl conversion error: {e}")))?;
        let record_as_provider = ProviderRecord {
            name: row.name.clone(),
            record_type: parse_record_type(&row.record_type)?,
            ttl,
            values,
        };

        // Use last observed DNS value as before_value when available (best snapshot
        // for rollback). Fall back to the desired record's own data.
        let observed_before: Option<String> = sqlx::query_scalar(
            "SELECT last_observed_value FROM sync_states
             WHERE record_id = ? AND last_observed_value IS NOT NULL
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(&row.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        .flatten();
        let before_value: Option<String> = if let Some(observed) = observed_before {
            Some(observed)
        } else {
            Some(serde_json::to_string(&record_as_provider).map_err(|e| {
                ApiError::Internal(format!("before_value serialization failed: {e}"))
            })?)
        };

        sqlx::query(
            "INSERT INTO changeset_items
                 (changeset_id, record_id, operation, before_value, after_value)
             VALUES (?, ?, 'delete', ?, NULL)",
        )
        .bind(&changeset_id)
        .bind(&row.id)
        .bind(&before_value)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }

    // Mark creates/updates as synced.
    if !pending_creates.is_empty() {
        sqlx::query(
            "UPDATE desired_records
             SET status = 'synced', updated_at = ?
             WHERE zone_id = ? AND status = 'pending' AND deleted_at IS NULL",
        )
        .bind(&now)
        .bind(&zone_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }

    // Mark pending deletes as synced — they will no longer appear in list_records.
    for row in &pending_deletes {
        sqlx::query(
            "UPDATE desired_records SET status = 'synced', updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(&row.id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }

    tx.commit().await.map_err(db_err)?;

    Ok((
        StatusCode::CREATED,
        Json(PushPendingResponse { changeset_id, record_count }),
    ))
}

// ── zone CRUD ─────────────────────────────────────────────────────────────────

/// GET /api/v1/zones/{zone_id} — fetch a single zone.
pub async fn get_zone(
    State(pool): State<Arc<DbPool>>,
    Path(zone_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let row = sqlx::query_as::<_, ZoneRow>(
        "SELECT id, name, default_ttl, owner_team_id, created_at, updated_at
         FROM zones WHERE id = ?",
    )
    .bind(&zone_id)
    .fetch_optional(&*pool)
    .await
    .map_err(db_err)?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(Zone {
        id: row.id,
        name: row.name,
        default_ttl: row.default_ttl,
        owner_team_id: row.owner_team_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// PATCH /api/v1/zones/{zone_id} — update name and/or default_ttl.
pub async fn update_zone(
    State(pool): State<Arc<DbPool>>,
    Path(zone_id): Path<String>,
    Json(body): Json<UpdateZoneRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.name.is_none() && body.default_ttl.is_none() {
        return Err(ApiError::UnprocessableEntity(
            "at least one of name or default_ttl must be provided".into(),
        ));
    }
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(ApiError::UnprocessableEntity(
                "name must not be empty".into(),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE zones
         SET name        = COALESCE(?, name),
             default_ttl = COALESCE(?, default_ttl),
             updated_at  = ?
         WHERE id = ?",
    )
    .bind(body.name.as_deref())
    .bind(body.default_ttl)
    .bind(&now)
    .bind(&zone_id)
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

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    let row = sqlx::query_as::<_, ZoneRow>(
        "SELECT id, name, default_ttl, owner_team_id, created_at, updated_at
         FROM zones WHERE id = ?",
    )
    .bind(&zone_id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;

    Ok(Json(Zone {
        id: row.id,
        name: row.name,
        default_ttl: row.default_ttl,
        owner_team_id: row.owner_team_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }))
}

/// DELETE /api/v1/zones/{zone_id} — delete a zone and cascade to all child rows.
pub async fn delete_zone(
    State(pool): State<Arc<DbPool>>,
    Path(zone_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM zones WHERE id = ?")
        .bind(&zone_id)
        .fetch_one(&*pool)
        .await
        .map_err(db_err)?;
    if count == 0 {
        return Err(ApiError::NotFound);
    }

    let mut tx = pool.begin().await.map_err(db_err)?;

    sqlx::query(
        "DELETE FROM sync_states
         WHERE record_id IN (SELECT id FROM desired_records WHERE zone_id = ?)",
    )
    .bind(&zone_id)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    sqlx::query(
        "DELETE FROM sync_states
         WHERE provider_binding_id IN (SELECT id FROM provider_bindings WHERE zone_id = ?)",
    )
    .bind(&zone_id)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    sqlx::query("DELETE FROM desired_records WHERE zone_id = ?")
        .bind(&zone_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    sqlx::query("DELETE FROM provider_bindings WHERE zone_id = ?")
        .bind(&zone_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    sqlx::query("DELETE FROM zones WHERE id = ?")
        .bind(&zone_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    tx.commit().await.map_err(db_err)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── record CRUD ───────────────────────────────────────────────────────────────

/// PATCH /api/v1/zones/{zone_id}/records/{record_id} — update ttl and/or values.
pub async fn update_record(
    State(pool): State<Arc<DbPool>>,
    Path((zone_id, record_id)): Path<(String, String)>,
    Json(body): Json<UpdateRecordRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.ttl.is_none() && body.values.is_none() {
        return Err(ApiError::UnprocessableEntity(
            "at least one of ttl or values must be provided".into(),
        ));
    }
    if let Some(ttl) = body.ttl {
        if ttl <= 0 {
            return Err(ApiError::BadRequest("ttl must be greater than 0".into()));
        }
    }
    if let Some(ref values) = body.values {
        if values.is_empty() {
            return Err(ApiError::UnprocessableEntity(
                "values must not be empty".into(),
            ));
        }
    }

    let current = sqlx::query_as::<_, RecordRow>(
        "SELECT id, zone_id, name, record_type, ttl, record_values, desired_hash, status,
                deleted_at, created_at, updated_at
         FROM desired_records
         WHERE id = ? AND zone_id = ? AND deleted_at IS NULL",
    )
    .bind(&record_id)
    .bind(&zone_id)
    .fetch_optional(&*pool)
    .await
    .map_err(db_err)?
    .ok_or(ApiError::NotFound)?;

    let new_ttl = body.ttl.unwrap_or(current.ttl);
    let new_values: Vec<String> = match body.values {
        Some(v) => v,
        None => serde_json::from_str(&current.record_values)
            .map_err(|e| ApiError::Internal(format!("record_values parse error: {e}")))?,
    };
    let new_values_json = serde_json::to_string(&new_values)
        .map_err(|e| ApiError::Internal(format!("values serialization failed: {e}")))?;
    let new_hash =
        compute_desired_hash(&current.name, &current.record_type, new_ttl, &new_values);
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE desired_records
         SET ttl = ?, record_values = ?, desired_hash = ?, status = 'pending', updated_at = ?
         WHERE id = ?",
    )
    .bind(new_ttl)
    .bind(&new_values_json)
    .bind(&new_hash)
    .bind(&now)
    .bind(&record_id)
    .execute(&*pool)
    .await
    .map_err(db_err)?;

    let row = sqlx::query_as::<_, RecordRow>(
        "SELECT id, zone_id, name, record_type, ttl, record_values, desired_hash, status,
                deleted_at, created_at, updated_at
         FROM desired_records WHERE id = ?",
    )
    .bind(&record_id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;

    Ok(Json(row.into_record()?))
}

/// DELETE /api/v1/zones/{zone_id}/bindings/{binding_id}
pub async fn delete_binding(
    State(pool): State<Arc<DbPool>>,
    Path((zone_id, binding_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_bindings WHERE id = ? AND zone_id = ?",
    )
    .bind(&binding_id)
    .bind(&zone_id)
    .fetch_one(&*pool)
    .await
    .map_err(db_err)?;
    if count == 0 {
        return Err(ApiError::NotFound);
    }

    let mut tx = pool.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM sync_states WHERE provider_binding_id = ?")
        .bind(&binding_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM provider_bindings WHERE id = ?")
        .bind(&binding_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── zone sync states ──────────────────────────────────────────────────────────

#[derive(FromRow)]
struct ZoneSyncStateRow {
    record_id: String,
    provider_binding_id: String,
    provider_name: String,
    last_observed_at: Option<String>,
    status: String,
    last_error: Option<String>,
    retry_count: i64,
}

#[derive(serde::Serialize)]
struct ZoneSyncState {
    record_id: String,
    provider_binding_id: String,
    provider_name: String,
    last_observed_at: Option<String>,
    status: String,
    last_error: Option<String>,
    retry_count: i64,
}

#[derive(serde::Serialize)]
struct ListZoneSyncStatesResponse {
    sync_states: Vec<ZoneSyncState>,
}

/// GET /api/v1/zones/{zone_id}/sync-states — list worker observation results for a zone.
pub async fn list_zone_sync_states(
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

    let rows = sqlx::query_as::<_, ZoneSyncStateRow>(
        "SELECT ss.record_id, ss.provider_binding_id, p.name AS provider_name,
                ss.last_observed_at, ss.status, ss.last_error, ss.retry_count
         FROM sync_states ss
         JOIN provider_bindings pb ON ss.provider_binding_id = pb.id
         JOIN providers p ON pb.provider_id = p.id
         WHERE pb.zone_id = ?
         ORDER BY ss.record_id, p.name",
    )
    .bind(&zone_id)
    .fetch_all(&*pool)
    .await
    .map_err(db_err)?;

    let sync_states = rows
        .into_iter()
        .map(|r| ZoneSyncState {
            record_id: r.record_id,
            provider_binding_id: r.provider_binding_id,
            provider_name: r.provider_name,
            last_observed_at: r.last_observed_at,
            status: r.status,
            last_error: r.last_error,
            retry_count: r.retry_count,
        })
        .collect();

    Ok(Json(ListZoneSyncStatesResponse { sync_states }))
}

/// DELETE /api/v1/zones/{zone_id}/records/{record_id}
///
/// If the record has never been pushed to DNS (`status = 'pending'`), it is
/// physically removed.  If it was already pushed (`status = 'synced'`), it is
/// soft-deleted (tombstone via `deleted_at`) so that the next
/// `POST /zones/{id}/push` creates a DNS delete changeset item.
pub async fn delete_record(
    State(pool): State<Arc<DbPool>>,
    Path((zone_id, record_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    #[derive(FromRow)]
    struct StatusRow {
        status: String,
    }

    let row = sqlx::query_as::<_, StatusRow>(
        "SELECT status FROM desired_records WHERE id = ? AND zone_id = ? AND deleted_at IS NULL",
    )
    .bind(&record_id)
    .bind(&zone_id)
    .fetch_optional(&*pool)
    .await
    .map_err(db_err)?
    .ok_or(ApiError::NotFound)?;

    let now = Utc::now().to_rfc3339();

    if row.status == "pending" {
        // Never pushed to DNS — remove from DB entirely.
        let mut tx = pool.begin().await.map_err(db_err)?;
        sqlx::query("DELETE FROM sync_states WHERE record_id = ?")
            .bind(&record_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM desired_records WHERE id = ?")
            .bind(&record_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
    } else {
        // Already in DNS — soft-delete. `push_pending_records` will create a
        // 'delete' changeset item on the next "Set to DNS" call.
        sqlx::query(
            "UPDATE desired_records SET deleted_at = ?, status = 'pending', updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(&record_id)
        .execute(&*pool)
        .await
        .map_err(db_err)?;
    }

    Ok(StatusCode::NO_CONTENT)
}
