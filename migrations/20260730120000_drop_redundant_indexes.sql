-- Drop incorrect global unique index on message_id alone.
-- The correct constraint is uc_cached_files_message_id_chat_id (composite, on message_id + chat_id),
-- which remains untouched.
DROP INDEX IF EXISTS ix_cached_files_message_id;

-- Drop redundant single-column indexes on object_id and object_type.
-- Every real query filters on (object_id, object_type, is_normalized), which is already
-- covered by the uc_cached_files_object_id_object_type_is_normalized unique constraint's index.
DROP INDEX IF EXISTS ix_cached_files_object_id;
DROP INDEX IF EXISTS ix_cached_files_object_type;
