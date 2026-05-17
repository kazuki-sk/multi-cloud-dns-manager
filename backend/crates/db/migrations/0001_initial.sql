-- Migration 0001: initial schema
--
-- All primary/foreign keys use TEXT (UUID stored as lowercase hyphenated string).
-- Timestamps use TEXT (ISO-8601 / RFC-3339, e.g. "2024-01-15T12:34:56Z").
-- This subset is valid for both SQLite and PostgreSQL.
--
-- Encrypted credential columns (credentials_blob, credentials_dek) store
-- base64-encoded AES-256-GCM ciphertext.  Plain-text credentials MUST NOT
-- be written to these columns (design principle §8.6).

-- ─── zones ────────────────────────────────────────────────────────────────────
-- Logical DNS zone managed by this tool (§5.2 Zone).
CREATE TABLE zones (
    id            TEXT    NOT NULL PRIMARY KEY,
    name          TEXT    NOT NULL UNIQUE,  -- e.g. "example.com"
    default_ttl   INTEGER NOT NULL DEFAULT 300,
    owner_team_id TEXT,                     -- FK → teams.id (table added later)
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
);

CREATE INDEX idx_zones_owner_team ON zones (owner_team_id);

-- ─── provider_bindings ────────────────────────────────────────────────────────
-- Mapping between a logical zone and one provider's zone (§5.2 ProviderBinding).
--
-- credentials_blob : AES-256-GCM ciphertext of the provider credential JSON
-- credentials_dek  : DEK (Data Encryption Key) encrypted with the KEK
--                    Both are base64-encoded; the KEK is injected via env var.
CREATE TABLE provider_bindings (
    id               TEXT NOT NULL PRIMARY KEY,
    zone_id          TEXT NOT NULL,
    provider_type    TEXT NOT NULL,   -- 'route53' | 'cloudflare' | 'gcloud' | …
    provider_zone_id TEXT NOT NULL,   -- provider-internal zone ID (e.g. "Z1234ABC")
    credentials_blob TEXT NOT NULL,   -- encrypted provider credentials (base64)
    credentials_dek  TEXT NOT NULL,   -- encrypted DEK for envelope encryption (base64)
    status           TEXT NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active', 'paused', 'error')),
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    FOREIGN KEY (zone_id) REFERENCES zones (id)
);

CREATE INDEX idx_provider_bindings_zone_id ON provider_bindings (zone_id);

-- ─── desired_records ──────────────────────────────────────────────────────────
-- Desired state: what DNS records *should* exist (§5.2 Record).
--
-- record_values : JSON array of record value strings
--                 MX example: ["10 mail.example.com.", "20 mail2.example.com."]
-- desired_hash  : SHA-256 hex of (name + type + ttl + canonical record_values),
--                 used for fast drift comparison in the Observe Phase.
-- deleted_at    : NULL → active record; non-NULL → tombstone (§3.6).
CREATE TABLE desired_records (
    id            TEXT    NOT NULL PRIMARY KEY,
    zone_id       TEXT    NOT NULL,
    name          TEXT    NOT NULL,   -- relative name: "www", "@", "*"
    record_type   TEXT    NOT NULL    -- MVP types + extension types (§7.1)
                      CHECK (record_type IN ('A', 'AAAA', 'CNAME', 'MX', 'TXT', 'NS', 'SRV', 'CAA')),
    record_values TEXT    NOT NULL,   -- JSON array
    ttl          INTEGER NOT NULL,
    desired_hash TEXT    NOT NULL,
    deleted_at   TEXT,               -- NULL = active; set = tombstone
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    FOREIGN KEY (zone_id) REFERENCES zones (id)
);

CREATE INDEX idx_desired_records_zone_id   ON desired_records (zone_id);
CREATE INDEX idx_desired_records_zone_name ON desired_records (zone_id, name, record_type);

-- Unique constraint on active records: one (zone, name, type) combination allowed.
-- Partial indexes are supported in SQLite ≥ 3.8.9 and all modern PostgreSQL.
CREATE UNIQUE INDEX idx_desired_records_unique_active
    ON desired_records (zone_id, name, record_type)
    WHERE deleted_at IS NULL;

-- ─── changesets ───────────────────────────────────────────────────────────────
-- Transactional unit of one or more record changes (§5.2 ChangeSet).
--
-- Status machine (§5.3):
--   draft → validated → applying → applied
--                             ↓
--                       rolling_back → rolled_back
--                             ↓
--                       rollback_failed  (frozen)
--                       frozen           (human intervention required)
CREATE TABLE changesets (
    id              TEXT NOT NULL PRIMARY KEY,
    created_by      TEXT NOT NULL,   -- user ID; FK → users.id (table added later)
    description     TEXT,
    status          TEXT NOT NULL DEFAULT 'draft'
                        CHECK (status IN (
                            'draft',
                            'validated',
                            'applying',
                            'applied',
                            'rolling_back',
                            'rolled_back',
                            'rollback_failed',
                            'frozen'
                        )),
    rollback_policy TEXT NOT NULL DEFAULT 'auto'
                        CHECK (rollback_policy IN ('auto', 'manual', 'frozen_on_failure')),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Worker queries changesets by status (e.g. find 'applying' on crash recovery §5.6).
CREATE INDEX idx_changesets_status     ON changesets (status);
CREATE INDEX idx_changesets_created_by ON changesets (created_by);

-- ─── changeset_items ──────────────────────────────────────────────────────────
-- Individual record-level change within a changeset (§5.2 ChangeSetItem).
--
-- before_value : JSON snapshot of the record before the change.
--                NULL for 'create' operations (no prior state).
-- after_value  : JSON snapshot of the record after the change.
--                NULL for 'delete' operations.
-- Both columns are used by the Rollback Phase to restore previous state.
CREATE TABLE changeset_items (
    changeset_id TEXT NOT NULL,
    record_id    TEXT NOT NULL,
    operation    TEXT NOT NULL
                     CHECK (operation IN ('create', 'update', 'delete')),
    before_value TEXT,   -- JSON; NULL for 'create'
    after_value  TEXT,   -- JSON; NULL for 'delete'
    PRIMARY KEY (changeset_id, record_id),
    FOREIGN KEY (changeset_id) REFERENCES changesets (id),
    FOREIGN KEY (record_id)    REFERENCES desired_records (id)
);

CREATE INDEX idx_changeset_items_record_id ON changeset_items (record_id);

-- ─── sync_states ──────────────────────────────────────────────────────────────
-- Actual state per (record × provider_binding) pair (§5.2 SyncState).
--
-- Populated and updated by the Observe Phase of the Reconcile Loop (§5.4).
-- status values:
--   in_sync     — desired == actual
--   drift       — desired != actual; alert raised, no auto-overwrite (principle §4)
--   sync_failed — provider query failed
--   syncing     — observe in progress
CREATE TABLE sync_states (
    record_id           TEXT    NOT NULL,
    provider_binding_id TEXT    NOT NULL,
    last_observed_value TEXT,           -- JSON of the actual record; NULL if never observed
    last_observed_at    TEXT,           -- ISO-8601 timestamp; NULL if never observed
    status              TEXT    NOT NULL DEFAULT 'syncing'
                            CHECK (status IN ('in_sync', 'drift', 'sync_failed', 'syncing')),
    last_error          TEXT,           -- error message on sync_failed
    retry_count         INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    PRIMARY KEY (record_id, provider_binding_id),
    FOREIGN KEY (record_id)           REFERENCES desired_records (id),
    FOREIGN KEY (provider_binding_id) REFERENCES provider_bindings (id)
);

-- Retry Phase queries by status (e.g. find all 'sync_failed' entries §5.4).
CREATE INDEX idx_sync_states_status              ON sync_states (status);
CREATE INDEX idx_sync_states_provider_binding_id ON sync_states (provider_binding_id);
