-- Add created_at to support cache-entry staleness/TTL semantics.
-- See docs/specs/05-cache-consistency-and-races.md (05.4).
ALTER TABLE cached_files
    ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now();
