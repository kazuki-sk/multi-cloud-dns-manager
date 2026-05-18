-- Migration 0003: add status column to desired_records
--
-- 'pending' = staged locally, not yet submitted to any DNS provider changeset.
-- 'synced'  = has been included in a validated changeset (push was initiated).
--
-- Existing rows get 'synced' so they are not incorrectly re-queued.
ALTER TABLE desired_records
    ADD COLUMN status TEXT NOT NULL DEFAULT 'synced'
        CHECK (status IN ('pending', 'synced'));
