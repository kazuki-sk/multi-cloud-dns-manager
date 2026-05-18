-- Migration 0002: split provider_bindings into providers + provider_bindings
--
-- Before: provider_bindings stored the zone mapping AND cloud credentials together.
-- After:
--   providers       → cloud account (zone-independent, holds encrypted credentials)
--   provider_bindings → zone ↔ provider mapping (no credentials)

-- Rename old table so we can reuse the name with the new schema.
ALTER TABLE provider_bindings RENAME TO provider_bindings_old;

-- ─── providers ────────────────────────────────────────────────────────────────
-- Cloud account (zone-independent).  Credentials are envelope-encrypted.
CREATE TABLE providers (
    id               TEXT NOT NULL PRIMARY KEY,
    name             TEXT NOT NULL,
    provider_type    TEXT NOT NULL,
    credentials_blob TEXT NOT NULL,
    credentials_dek  TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active', 'paused', 'error')),
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

-- ─── provider_bindings ────────────────────────────────────────────────────────
-- Maps a logical zone to one provider zone.  No credentials stored here.
CREATE TABLE provider_bindings (
    id               TEXT NOT NULL PRIMARY KEY,
    zone_id          TEXT NOT NULL,
    provider_id      TEXT NOT NULL,
    provider_zone_id TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'active'
                         CHECK (status IN ('active', 'paused', 'error')),
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    FOREIGN KEY (zone_id)     REFERENCES zones (id),
    FOREIGN KEY (provider_id) REFERENCES providers (id)
);

CREATE INDEX idx_pb_zone_id     ON provider_bindings (zone_id);
CREATE INDEX idx_pb_provider_id ON provider_bindings (provider_id);

-- Drop old table (data is not migrated; this is a dev-environment schema change).
DROP TABLE IF EXISTS provider_bindings_old;
