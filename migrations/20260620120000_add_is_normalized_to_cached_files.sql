-- Add is_normalized column to cached_files to support multiple cache entries
-- per (object_id, object_type) tuple, one per is_normalized mode.
-- Existing rows default to TRUE (previously all entries were implicitly normalized).

ALTER TABLE cached_files
    ADD COLUMN IF NOT EXISTS is_normalized BOOLEAN NOT NULL DEFAULT TRUE;

-- Replace the old unique constraint with one that includes is_normalized.
-- This allows two cache entries for the same (object_id, object_type):
-- one for normalized=true, one for normalized=false.

ALTER TABLE cached_files
    DROP CONSTRAINT IF EXISTS uc_cached_files_object_id_object_type;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'uc_cached_files_object_id_object_type_is_normalized'
    ) THEN
        ALTER TABLE cached_files
        ADD CONSTRAINT uc_cached_files_object_id_object_type_is_normalized
        UNIQUE (object_id, object_type, is_normalized);
    END IF;
END $$;
