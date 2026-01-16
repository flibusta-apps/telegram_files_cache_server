-- Create cached_files table with all indexes and constraints
-- This migration is idempotent and safe to run on existing databases

-- Create table if not exists
CREATE TABLE IF NOT EXISTS cached_files (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT nextval('cached_files_id_seq'::regclass),
    object_id INTEGER NOT NULL,
    object_type VARCHAR(8) NOT NULL,
    message_id BIGINT NOT NULL,
    chat_id BIGINT NOT NULL
);

-- Create sequence if not exists (for compatibility)
CREATE SEQUENCE IF NOT EXISTS cached_files_id_seq;

-- Ensure the sequence is owned by the table column
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_depend
        WHERE refobjid = 'cached_files'::regclass
        AND objid = 'cached_files_id_seq'::regclass
    ) THEN
        ALTER SEQUENCE cached_files_id_seq OWNED BY cached_files.id;
    END IF;
END $$;

-- Create indexes if they don't exist
CREATE UNIQUE INDEX IF NOT EXISTS ix_cached_files_message_id ON cached_files (message_id);
CREATE INDEX IF NOT EXISTS ix_cached_files_object_id ON cached_files (object_id);
CREATE INDEX IF NOT EXISTS ix_cached_files_object_type ON cached_files (object_type);

-- Create unique constraints if they don't exist
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'uc_cached_files_message_id_chat_id'
    ) THEN
        ALTER TABLE cached_files
        ADD CONSTRAINT uc_cached_files_message_id_chat_id UNIQUE (message_id, chat_id);
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'uc_cached_files_object_id_object_type'
    ) THEN
        ALTER TABLE cached_files
        ADD CONSTRAINT uc_cached_files_object_id_object_type UNIQUE (object_id, object_type);
    END IF;
END $$;
