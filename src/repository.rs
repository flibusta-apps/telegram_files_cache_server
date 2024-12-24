use crate::{serializers::CachedFile, views::Database};

pub struct CachedFileRepository {
    db: Database,
}

impl CachedFileRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn delete_by_object_id_object_type(
        &self,
        object_id: i32,
        object_type: String,
    ) -> Result<CachedFile, sqlx::Error> {
        sqlx::query_as!(
            CachedFile,
            r#"
            DELETE FROM cached_files
            WHERE object_id = $1 AND object_type = $2
            RETURNING *
            "#,
            object_id,
            object_type
        )
        .fetch_one(&self.db)
        .await
    }
}
