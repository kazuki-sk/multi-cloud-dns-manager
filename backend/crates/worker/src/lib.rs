use std::collections::HashMap;
use std::sync::Arc;

use dns_manager_core::{
    diff, ChangeSetStatus, Credentials, EncryptedBlob, KeyProvider, ProviderAdapter,
    ProviderRecord, RecordKey, RecordType,
};
use dns_manager_db::DbPool;
use sqlx::FromRow;
use tokio::time::{interval, Duration, Instant};

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

/// Update a changeset's status, enforcing `can_transition_to`.
async fn update_changeset_status(
    db: &DbPool,
    changeset_id: &str,
    from: ChangeSetStatus,
    to: ChangeSetStatus,
) -> anyhow::Result<()> {
    if !from.can_transition_to(to) {
        anyhow::bail!(
            "invalid state transition for {changeset_id}: {} → {}",
            status_to_db(from),
            status_to_db(to)
        );
    }
    let now = chrono::Utc::now().to_rfc3339();
    let affected = sqlx::query(
        "UPDATE changesets SET status = ?, updated_at = ? WHERE id = ? AND status = ?",
    )
    .bind(status_to_db(to))
    .bind(&now)
    .bind(changeset_id)
    .bind(status_to_db(from))
    .execute(db)
    .await?
    .rows_affected();

    if affected == 0 {
        anyhow::bail!(
            "changeset {changeset_id} was not in expected status '{}' (already processed?)",
            status_to_db(from)
        );
    }
    Ok(())
}

// ── DB row types ──────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct ChangesetRow {
    id: String,
    rollback_policy: String,
}

#[derive(FromRow)]
struct ChangesetItemRow {
    record_id: String,
    operation: String,
    before_value: Option<String>,
    after_value: Option<String>,
}

#[derive(FromRow)]
struct DesiredRecordZoneRow {
    zone_id: String,
}

#[derive(FromRow)]
struct ProviderBindingRow {
    id: String,
    provider_type: String,
    provider_zone_id: String,
    credentials_blob: String,
    credentials_dek: String,
}

#[derive(FromRow)]
struct DesiredRecordRow {
    id: String,
    name: String,
    record_type: String,
    record_values: String, // JSON array
    ttl: i64,
}

#[derive(FromRow)]
struct SyncStateFailedRow {
    record_id: String,
    provider_binding_id: String,
    last_observed_at: Option<String>,
    retry_count: i64,
}

// ── apply context (kept for rollback) ─────────────────────────────────────────

struct AppliedItemCtx {
    zone_id: String,
    operation: String,
    before_record: Option<ProviderRecord>,
    after_record: Option<ProviderRecord>,
}

// ── worker ────────────────────────────────────────────────────────────────────

pub struct ReconcileWorker {
    pub db: Arc<DbPool>,
    pub adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
    pub key_provider: Arc<dyn KeyProvider>,
}

impl ReconcileWorker {
    pub fn new(
        db: Arc<DbPool>,
        adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
        key_provider: Arc<dyn KeyProvider>,
    ) -> Self {
        Self { db, adapters, key_provider }
    }

    pub async fn run(self: Arc<Self>) {
        // Reset changesets stuck in 'applying' from a previous crash.
        // Safe because all adapter operations are required to be idempotent (design §5.4).
        if let Err(e) = self.startup_recovery().await {
            tracing::error!(error = %e, "startup recovery failed");
        }

        let h1 = tokio::spawn({
            let w = Arc::clone(&self);
            async move { w.run_apply_loop().await }
        });
        let h2 = tokio::spawn({
            let w = Arc::clone(&self);
            async move { w.run_observe_loop().await }
        });
        let h3 = tokio::spawn({
            let w = Arc::clone(&self);
            async move { w.run_retry_loop().await }
        });
        let _ = tokio::join!(h1, h2, h3);
    }

    async fn startup_recovery(&self) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let count = sqlx::query(
            "UPDATE changesets SET status = 'validated', updated_at = ? WHERE status = 'applying'",
        )
        .bind(&now)
        .execute(&*self.db)
        .await?
        .rows_affected();

        if count > 0 {
            tracing::warn!(
                count,
                "startup: reset {} stuck 'applying' changeset(s) to 'validated' for reprocessing",
                count
            );
        }
        Ok(())
    }

    // ── apply loop ────────────────────────────────────────────────────────────

    async fn run_apply_loop(self: Arc<Self>) {
        let mut ticker = interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;
            if let Err(e) = self.process_apply_batch().await {
                tracing::error!(error = %e, "apply loop: batch processing failed");
            }
        }
    }

    async fn process_apply_batch(&self) -> anyhow::Result<()> {
        let rows: Vec<ChangesetRow> = sqlx::query_as(
            "SELECT id, rollback_policy FROM changesets
             WHERE status = 'validated'
             ORDER BY created_at",
        )
        .fetch_all(&*self.db)
        .await?;

        for row in rows {
            if let Err(e) = self.apply_single_changeset(&row.id, &row.rollback_policy).await {
                tracing::error!(changeset_id = %row.id, error = %e, "apply_single_changeset returned error");
            }
        }
        Ok(())
    }

    async fn apply_single_changeset(
        &self,
        changeset_id: &str,
        rollback_policy: &str,
    ) -> anyhow::Result<()> {
        let start = Instant::now();

        update_changeset_status(
            &self.db,
            changeset_id,
            ChangeSetStatus::Validated,
            ChangeSetStatus::Applying,
        )
        .await?;

        let items: Vec<ChangesetItemRow> = sqlx::query_as(
            "SELECT record_id, operation, before_value, after_value
             FROM changeset_items WHERE changeset_id = ?
             ORDER BY rowid",
        )
        .bind(changeset_id)
        .fetch_all(&*self.db)
        .await?;

        match self.apply_items(changeset_id, &items).await {
            Ok(applied) => {
                update_changeset_status(
                    &self.db,
                    changeset_id,
                    ChangeSetStatus::Applying,
                    ChangeSetStatus::Applied,
                )
                .await?;
                tracing::info!(changeset_id, items = applied.len(), "changeset applied successfully");
            }
            Err((applied, err)) => {
                let elapsed = start.elapsed();
                tracing::error!(
                    changeset_id,
                    error = %err,
                    elapsed_ms = elapsed.as_millis(),
                    "changeset apply failed",
                );

                let auto_rollback =
                    rollback_policy == "auto" && elapsed < Duration::from_secs(30);

                if !auto_rollback {
                    if let Err(e) = update_changeset_status(
                        &self.db,
                        changeset_id,
                        ChangeSetStatus::Applying,
                        ChangeSetStatus::Frozen,
                    )
                    .await
                    {
                        tracing::error!(changeset_id, error = %e, "failed to transition to frozen");
                    }
                    return Ok(());
                }

                if let Err(e) = update_changeset_status(
                    &self.db,
                    changeset_id,
                    ChangeSetStatus::Applying,
                    ChangeSetStatus::RollingBack,
                )
                .await
                {
                    tracing::error!(changeset_id, error = %e, "failed to transition to rolling_back");
                    return Ok(());
                }

                match self.rollback_items(&applied).await {
                    Ok(()) => {
                        if let Err(e) = update_changeset_status(
                            &self.db,
                            changeset_id,
                            ChangeSetStatus::RollingBack,
                            ChangeSetStatus::RolledBack,
                        )
                        .await
                        {
                            tracing::error!(changeset_id, error = %e, "failed to transition to rolled_back");
                        } else {
                            tracing::info!(changeset_id, "changeset rolled back successfully");
                        }
                    }
                    Err(rb_err) => {
                        tracing::error!(changeset_id, error = %rb_err, "rollback failed");
                        if let Err(e) = update_changeset_status(
                            &self.db,
                            changeset_id,
                            ChangeSetStatus::RollingBack,
                            ChangeSetStatus::RollbackFailed,
                        )
                        .await
                        {
                            tracing::error!(changeset_id, error = %e, "failed to transition to rollback_failed");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn apply_items(
        &self,
        changeset_id: &str,
        items: &[ChangesetItemRow],
    ) -> Result<Vec<AppliedItemCtx>, (Vec<AppliedItemCtx>, anyhow::Error)> {
        let mut applied: Vec<AppliedItemCtx> = Vec::new();
        for item in items {
            match self.apply_one_item(item).await {
                Ok(ctx) => applied.push(ctx),
                Err(e) => {
                    tracing::error!(
                        changeset_id,
                        record_id = %item.record_id,
                        operation = %item.operation,
                        error = %e,
                        "item apply failed",
                    );
                    return Err((applied, e));
                }
            }
        }
        Ok(applied)
    }

    async fn apply_one_item(&self, item: &ChangesetItemRow) -> anyhow::Result<AppliedItemCtx> {
        let row: DesiredRecordZoneRow =
            sqlx::query_as("SELECT zone_id FROM desired_records WHERE id = ?")
                .bind(&item.record_id)
                .fetch_optional(&*self.db)
                .await?
                .ok_or_else(|| anyhow::anyhow!("desired_record '{}' not found", item.record_id))?;
        let zone_id = row.zone_id;

        let after_record: Option<ProviderRecord> = item
            .after_value
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| anyhow::anyhow!("after_value parse error: {e}"))?;

        let before_record: Option<ProviderRecord> = item
            .before_value
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| anyhow::anyhow!("before_value parse error: {e}"))?;

        let bindings: Vec<ProviderBindingRow> = sqlx::query_as(
            "SELECT pb.id, p.provider_type, pb.provider_zone_id,
                    p.credentials_blob, p.credentials_dek
             FROM provider_bindings pb
             JOIN providers p ON pb.provider_id = p.id
             WHERE pb.zone_id = ? AND pb.status = 'active' AND p.status = 'active'",
        )
        .bind(&zone_id)
        .fetch_all(&*self.db)
        .await?;

        if bindings.is_empty() {
            anyhow::bail!("no active provider bindings for zone '{zone_id}'");
        }

        for binding in &bindings {
            let adapter =
                self.adapters.get(&binding.provider_type).ok_or_else(|| {
                    anyhow::anyhow!(
                        "no adapter registered for provider type '{}'",
                        binding.provider_type
                    )
                })?;
            let creds =
                self.decrypt_credentials(&binding.credentials_blob, &binding.credentials_dek)?;

            match item.operation.as_str() {
                "create" | "update" => {
                    let record = after_record.as_ref().ok_or_else(|| {
                        anyhow::anyhow!(
                            "after_value is required for '{}' operation",
                            item.operation
                        )
                    })?;
                    adapter
                        .upsert_record(&creds, &binding.provider_zone_id, record)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "upsert_record failed on '{}': {e}",
                                binding.provider_type
                            )
                        })?;
                }
                "delete" => {
                    let record = before_record.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("before_value is required for 'delete' operation")
                    })?;
                    let key = RecordKey::from(record);
                    adapter
                        .delete_record(&creds, &binding.provider_zone_id, &key)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "delete_record failed on '{}': {e}",
                                binding.provider_type
                            )
                        })?;
                }
                op => anyhow::bail!("unknown operation '{op}'"),
            }
        }

        Ok(AppliedItemCtx { zone_id, operation: item.operation.clone(), before_record, after_record })
    }

    async fn rollback_items(&self, applied: &[AppliedItemCtx]) -> anyhow::Result<()> {
        for ctx in applied.iter().rev() {
            let bindings: Vec<ProviderBindingRow> = sqlx::query_as(
                "SELECT pb.id, p.provider_type, pb.provider_zone_id,
                        p.credentials_blob, p.credentials_dek
                 FROM provider_bindings pb
                 JOIN providers p ON pb.provider_id = p.id
                 WHERE pb.zone_id = ? AND pb.status = 'active' AND p.status = 'active'",
            )
            .bind(&ctx.zone_id)
            .fetch_all(&*self.db)
            .await?;

            for binding in &bindings {
                let adapter =
                    self.adapters.get(&binding.provider_type).ok_or_else(|| {
                        anyhow::anyhow!(
                            "no adapter registered for provider type '{}' during rollback",
                            binding.provider_type
                        )
                    })?;
                let creds = self.decrypt_credentials(
                    &binding.credentials_blob,
                    &binding.credentials_dek,
                )?;

                match ctx.operation.as_str() {
                    "create" => {
                        if let Some(rec) = &ctx.after_record {
                            let key = RecordKey::from(rec);
                            adapter
                                .delete_record(&creds, &binding.provider_zone_id, &key)
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "rollback delete failed on '{}': {e}",
                                        binding.provider_type
                                    )
                                })?;
                        }
                    }
                    "update" | "delete" => {
                        if let Some(rec) = &ctx.before_record {
                            adapter
                                .upsert_record(&creds, &binding.provider_zone_id, rec)
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "rollback upsert failed on '{}': {e}",
                                        binding.provider_type
                                    )
                                })?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn decrypt_credentials(
        &self,
        credentials_blob: &str,
        credentials_dek: &str,
    ) -> anyhow::Result<Credentials> {
        let encrypted = EncryptedBlob {
            blob: credentials_blob.to_string(),
            dek: credentials_dek.to_string(),
        };
        let bytes = dns_manager_core::decrypt(&*self.key_provider, &encrypted)
            .map_err(|e| anyhow::anyhow!("credential decryption failed: {e}"))?;
        let map: HashMap<String, String> = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("credentials JSON parse failed: {e}"))?;
        Ok(Credentials(map))
    }

    // ── observe loop ──────────────────────────────────────────────────────────

    /// Poll every 300 s: for each active provider binding, fetch actual DNS
    /// records and compare against desired state.  Results are written to
    /// `sync_states` as `in_sync`, `drift`, or `sync_failed`.
    async fn run_observe_loop(self: Arc<Self>) {
        let mut ticker = interval(Duration::from_secs(300));
        loop {
            ticker.tick().await;
            if let Err(e) = self.process_observe_batch().await {
                tracing::error!(error = %e, "observe loop: batch processing failed");
            }
        }
    }

    async fn process_observe_batch(&self) -> anyhow::Result<()> {
        let bindings: Vec<ProviderBindingRow> = sqlx::query_as(
            "SELECT pb.id, p.provider_type, pb.provider_zone_id,
                    p.credentials_blob, p.credentials_dek
             FROM provider_bindings pb
             JOIN providers p ON pb.provider_id = p.id
             WHERE pb.status = 'active' AND p.status = 'active'",
        )
        .fetch_all(&*self.db)
        .await?;

        for binding in &bindings {
            // One binding's failure must not block the others.
            if let Err(e) = self.observe_one_binding(binding).await {
                tracing::error!(
                    provider_binding_id = %binding.id,
                    provider_type = %binding.provider_type,
                    error = %e,
                    "observe loop: binding observation failed",
                );
            }
        }
        Ok(())
    }

    async fn observe_one_binding(&self, binding: &ProviderBindingRow) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        // ── load desired records for this zone ────────────────────────────────
        let desired_rows: Vec<DesiredRecordRow> = sqlx::query_as(
            "SELECT id, name, record_type, record_values, ttl
             FROM desired_records
             WHERE zone_id = (SELECT zone_id FROM provider_bindings WHERE id = ?)
               AND deleted_at IS NULL
               AND status = 'synced'",
        )
        .bind(&binding.id)
        .fetch_all(&*self.db)
        .await?;

        if desired_rows.is_empty() {
            return Ok(());
        }

        // ── fetch actual state from provider ──────────────────────────────────
        let actual_result = self.fetch_actual_records(binding).await;

        // ── on provider failure: mark all desired records as sync_failed ──────
        let actual_map = match actual_result {
            Ok(records) => records,
            Err(provider_err) => {
                tracing::error!(
                    provider_binding_id = %binding.id,
                    error = %provider_err,
                    "observe loop: list_records failed",
                );
                let err_msg = provider_err.to_string();
                for row in &desired_rows {
                    upsert_sync_state(
                        &self.db,
                        &row.id,
                        &binding.id,
                        None,
                        Some(&now),
                        "sync_failed",
                        Some(&err_msg),
                        &now,
                    )
                    .await?;
                }
                return Ok(());
            }
        };

        // ── compare desired vs actual and write sync_states ───────────────────
        for row in &desired_rows {
            let desired = match parse_desired_record(row) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(
                        record_id = %row.id,
                        error = %e,
                        "observe loop: failed to parse desired record",
                    );
                    continue;
                }
            };

            let key = RecordKey::from(&desired);
            let actual = actual_map.get(&key);

            let (status, observed_json) = match actual {
                Some(actual_record) => {
                    let observed_json = serde_json::to_string(actual_record)
                        .map_err(|e| anyhow::anyhow!("actual record serialization failed: {e}"))?;
                    if diff(actual_record, &desired) {
                        tracing::warn!(
                            provider_binding_id = %binding.id,
                            record_name = %desired.name,
                            record_type = ?desired.record_type,
                            "observe loop: drift detected",
                        );
                        ("drift", Some(observed_json))
                    } else {
                        ("in_sync", Some(observed_json))
                    }
                }
                None => {
                    // Record should exist but is absent from the provider.
                    tracing::warn!(
                        provider_binding_id = %binding.id,
                        record_name = %desired.name,
                        record_type = ?desired.record_type,
                        "observe loop: drift detected (record missing from provider)",
                    );
                    ("drift", None)
                }
            };

            upsert_sync_state(
                &self.db,
                &row.id,
                &binding.id,
                observed_json.as_deref(),
                Some(&now),
                status,
                None,
                &now,
            )
            .await?;
        }

        Ok(())
    }

    /// Decrypt credentials and call `adapter.list_records`, returning a map
    /// keyed by `RecordKey` for O(1) lookup during comparison.
    async fn fetch_actual_records(
        &self,
        binding: &ProviderBindingRow,
    ) -> anyhow::Result<HashMap<RecordKey, ProviderRecord>> {
        let adapter = self.adapters.get(&binding.provider_type).ok_or_else(|| {
            anyhow::anyhow!(
                "no adapter registered for provider type '{}'",
                binding.provider_type
            )
        })?;
        let creds =
            self.decrypt_credentials(&binding.credentials_blob, &binding.credentials_dek)?;
        let records = adapter
            .list_records(&creds, &binding.provider_zone_id)
            .await
            .map_err(|e| anyhow::anyhow!("list_records failed: {e}"))?;

        Ok(records.into_iter().map(|r| (RecordKey::from(&r), r)).collect())
    }

    // ── retry loop ────────────────────────────────────────────────────────────

    /// Poll every 60 s for `sync_failed` sync states.  Each entry is retried
    /// according to exponential backoff (`2^retry_count` seconds, max 3600 s).
    async fn run_retry_loop(self: Arc<Self>) {
        let mut ticker = interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            if let Err(e) = self.process_retry_batch().await {
                tracing::error!(error = %e, "retry loop: batch processing failed");
            }
        }
    }

    async fn process_retry_batch(&self) -> anyhow::Result<()> {
        let rows: Vec<SyncStateFailedRow> = sqlx::query_as(
            "SELECT record_id, provider_binding_id, last_observed_at, retry_count
             FROM sync_states
             WHERE status = 'sync_failed'
             ORDER BY retry_count ASC, last_observed_at ASC",
        )
        .fetch_all(&*self.db)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now();

        // Filter to entries whose backoff window has elapsed, grouped by binding.
        let mut groups: HashMap<String, Vec<&SyncStateFailedRow>> = HashMap::new();
        for row in &rows {
            if should_retry(row, now) {
                groups.entry(row.provider_binding_id.clone()).or_default().push(row);
            }
        }

        if groups.is_empty() {
            return Ok(());
        }

        tracing::info!(bindings = groups.len(), "retry loop: retrying sync_failed bindings");

        for (binding_id, entries) in &groups {
            if let Err(e) = self.retry_one_binding(binding_id, entries).await {
                tracing::error!(
                    provider_binding_id = %binding_id,
                    error = %e,
                    "retry loop: binding retry failed",
                );
            }
        }
        Ok(())
    }

    /// Attempt to resolve all `sync_failed` entries for a single provider binding.
    /// `list_records()` is called once and the result is applied to every entry.
    async fn retry_one_binding(
        &self,
        binding_id: &str,
        entries: &[&SyncStateFailedRow],
    ) -> anyhow::Result<()> {
        let now_str = chrono::Utc::now().to_rfc3339();

        let binding = sqlx::query_as::<_, ProviderBindingRow>(
            "SELECT pb.id, p.provider_type, pb.provider_zone_id,
                    p.credentials_blob, p.credentials_dek
             FROM provider_bindings pb
             JOIN providers p ON pb.provider_id = p.id
             WHERE pb.id = ? AND pb.status = 'active' AND p.status = 'active'",
        )
        .bind(binding_id)
        .fetch_optional(&*self.db)
        .await?;
        let Some(binding) = binding else {
            tracing::info!(
                provider_binding_id = %binding_id,
                "retry loop: skipping inactive or missing binding",
            );
            return Ok(());
        };

        match self.fetch_actual_records(&binding).await {
            Ok(actual_map) => {
                for entry in entries {
                    if let Err(e) =
                        self.retry_resolve_entry(entry, &binding, &actual_map, &now_str).await
                    {
                        tracing::error!(
                            record_id = %entry.record_id,
                            error = %e,
                            "retry loop: record resolution failed",
                        );
                    }
                }
            }
            Err(provider_err) => {
                let err_msg = provider_err.to_string();
                tracing::warn!(
                    provider_binding_id = %binding_id,
                    error = %err_msg,
                    "retry loop: list_records failed again",
                );
                for entry in entries {
                    let next_count = entry.retry_count + 1;
                    if next_count > 10 {
                        tracing::warn!(
                            record_id = %entry.record_id,
                            provider_binding_id = %entry.provider_binding_id,
                            retry_count = next_count,
                            "retry loop: retry_count > 10 — manual intervention may be required",
                        );
                    }
                    // upsert_sync_state with sync_failed increments retry_count via SQL CASE.
                    if let Err(e) = upsert_sync_state(
                        &self.db,
                        &entry.record_id,
                        binding_id,
                        None,
                        Some(&now_str),
                        "sync_failed",
                        Some(&err_msg),
                        &now_str,
                    )
                    .await
                    {
                        tracing::error!(
                            record_id = %entry.record_id,
                            error = %e,
                            "retry loop: sync_state update failed",
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Compare one desired record against the fetched actual state and write
    /// the result (`in_sync` / `drift`) to `sync_states`.
    async fn retry_resolve_entry(
        &self,
        entry: &SyncStateFailedRow,
        binding: &ProviderBindingRow,
        actual_map: &HashMap<RecordKey, ProviderRecord>,
        now_str: &str,
    ) -> anyhow::Result<()> {
        let desired_row: DesiredRecordRow = sqlx::query_as(
            "SELECT id, name, record_type, record_values, ttl
             FROM desired_records WHERE id = ?",
        )
        .bind(&entry.record_id)
        .fetch_optional(&*self.db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("desired_record '{}' not found", entry.record_id))?;

        let desired = parse_desired_record(&desired_row)?;
        let key = RecordKey::from(&desired);

        let (status, observed_json) = match actual_map.get(&key) {
            Some(actual_record) => {
                let json = serde_json::to_string(actual_record)
                    .map_err(|e| anyhow::anyhow!("actual record serialization failed: {e}"))?;
                if diff(actual_record, &desired) {
                    tracing::warn!(
                        record_id = %entry.record_id,
                        provider_binding_id = %binding.id,
                        "retry loop: drift detected after retry",
                    );
                    ("drift", Some(json))
                } else {
                    ("in_sync", Some(json))
                }
            }
            None => {
                tracing::warn!(
                    record_id = %entry.record_id,
                    provider_binding_id = %binding.id,
                    "retry loop: drift detected after retry (record missing from provider)",
                );
                ("drift", None)
            }
        };

        tracing::info!(
            record_id = %entry.record_id,
            status,
            "retry loop: sync_failed resolved",
        );

        upsert_sync_state(
            &self.db,
            &entry.record_id,
            &binding.id,
            observed_json.as_deref(),
            Some(now_str),
            status,
            None,
            now_str,
        )
        .await
    }
}

// ── module-level helpers ──────────────────────────────────────────────────────

/// Exponential backoff: `min(2^retry_count, 3600)` seconds.
///
/// Shift is clamped to 63 to prevent u64 left-shift overflow.
fn backoff_secs(retry_count: i64) -> u64 {
    let shift = retry_count.max(0).min(63) as u32;
    (1u64 << shift).min(3600)
}

/// Returns `true` if enough time has elapsed since `last_observed_at` to retry.
fn should_retry(entry: &SyncStateFailedRow, now: chrono::DateTime<chrono::Utc>) -> bool {
    let backoff = backoff_secs(entry.retry_count);

    let Some(last_at_str) = &entry.last_observed_at else {
        return true; // never observed — retry immediately
    };

    let Ok(last_at) = chrono::DateTime::parse_from_rfc3339(last_at_str) else {
        return true; // unparseable timestamp — retry to be safe
    };

    let elapsed = now.signed_duration_since(last_at.with_timezone(&chrono::Utc));
    elapsed.num_seconds() >= backoff as i64
}

/// Parse a `DesiredRecordRow` from the DB into a `ProviderRecord`.
fn parse_desired_record(row: &DesiredRecordRow) -> anyhow::Result<ProviderRecord> {
    let values: Vec<String> = serde_json::from_str(&row.record_values)
        .map_err(|e| anyhow::anyhow!("record_values parse error for {}: {e}", row.id))?;
    // RecordType uses SCREAMING_SNAKE_CASE serde: wrap in quotes to deserialize.
    let record_type: RecordType = serde_json::from_str(&format!("\"{}\"", row.record_type))
        .map_err(|e| anyhow::anyhow!("record_type parse error '{}': {e}", row.record_type))?;
    let ttl = u32::try_from(row.ttl)
        .map_err(|e| anyhow::anyhow!("ttl conversion error for {}: {e}", row.id))?;
    Ok(ProviderRecord {
        name: row.name.clone(),
        record_type,
        ttl,
        values,
    })
}

/// Upsert a row into `sync_states`.
///
/// On conflict, preserves existing `created_at` and manages `retry_count`:
/// - Resolving a failure (non-sync_failed) → reset to 0.
/// - Continuing sync_failed → increment.
/// - New sync_failed (was not previously failed) → reset to 0.
async fn upsert_sync_state(
    db: &DbPool,
    record_id: &str,
    provider_binding_id: &str,
    last_observed_value: Option<&str>,
    last_observed_at: Option<&str>,
    status: &str,
    last_error: Option<&str>,
    now: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sync_states
             (record_id, provider_binding_id, last_observed_value, last_observed_at,
              status, last_error, retry_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)
         ON CONFLICT(record_id, provider_binding_id) DO UPDATE SET
             last_observed_value = excluded.last_observed_value,
             last_observed_at    = excluded.last_observed_at,
             status              = excluded.status,
             last_error          = excluded.last_error,
             retry_count         = CASE
                                     WHEN excluded.status != 'sync_failed' THEN 0
                                     WHEN sync_states.status = 'sync_failed'
                                          THEN sync_states.retry_count + 1
                                     ELSE 0
                                   END,
             updated_at          = excluded.updated_at",
    )
    .bind(record_id)
    .bind(provider_binding_id)
    .bind(last_observed_value)
    .bind(last_observed_at)
    .bind(status)
    .bind(last_error)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dns_manager_core::EnvKeyProvider;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

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

    #[tokio::test]
    async fn startup_recovery_resets_applying_to_validated() {
        let pool = make_pool().await;
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO changesets
                 (id, created_by, description, status, rollback_policy, created_at, updated_at)
             VALUES (?, ?, NULL, ?, 'auto', ?, ?)",
        )
        .bind("cs-applying")
        .bind("tester")
        .bind("applying")
        .bind(&now)
        .bind(&now)
        .execute(&*pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO changesets
                 (id, created_by, description, status, rollback_policy, created_at, updated_at)
             VALUES (?, ?, NULL, ?, 'auto', ?, ?)",
        )
        .bind("cs-validated")
        .bind("tester")
        .bind("validated")
        .bind(&now)
        .bind(&now)
        .execute(&*pool)
        .await
        .unwrap();

        let worker = ReconcileWorker::new(
            Arc::clone(&pool),
            HashMap::new(),
            Arc::new(EnvKeyProvider),
        );
        worker.startup_recovery().await.unwrap();

        let applying_status: String =
            sqlx::query_scalar("SELECT status FROM changesets WHERE id = ?")
                .bind("cs-applying")
                .fetch_one(&*pool)
                .await
                .unwrap();
        assert_eq!(applying_status, "validated");

        let validated_status: String =
            sqlx::query_scalar("SELECT status FROM changesets WHERE id = ?")
                .bind("cs-validated")
                .fetch_one(&*pool)
                .await
                .unwrap();
        assert_eq!(validated_status, "validated");
    }
}
