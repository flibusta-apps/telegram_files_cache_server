use prisma_client_rust::QueryError;

use crate::{prisma::cached_file, views::Database};

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
    ) -> Result<cached_file::Data, QueryError> {
        self.db
            .cached_file()
            .delete(cached_file::object_id_object_type(object_id, object_type))
            .exec()
            .await
    }
}
