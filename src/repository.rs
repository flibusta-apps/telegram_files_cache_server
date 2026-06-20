use crate::{serializers::CachedFile, views::Database};

pub struct CachedFileRepository {
    db: Database,
}

impl CachedFileRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn delete_by_object_id_object_type_is_normalized(
        &self,
        object_id: i32,
        object_type: String,
        is_normalized: bool,
    ) -> Result<CachedFile, sqlx::Error> {
        sqlx::query_as!(
            CachedFile,
            r#"
            DELETE FROM cached_files
            WHERE object_id = $1 AND object_type = $2 AND is_normalized = $3
            RETURNING *
            "#,
            object_id,
            object_type,
            is_normalized
        )
        .fetch_one(&self.db)
        .await
    }
}
