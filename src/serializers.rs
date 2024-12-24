#[derive(sqlx::FromRow, serde::Serialize)]
pub struct CachedFile {
    pub id: i32,
    pub object_id: i32,
    pub object_type: String,
    pub message_id: i64,
    pub chat_id: i64,
}
