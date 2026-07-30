use crate::config::CONFIG;

use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn get_pg_pool() -> PgPool {
    let database_url: String = format!(
        "postgresql://{}:{}@{}:{}/{}",
        CONFIG.postgres_user,
        CONFIG.postgres_password,
        CONFIG.postgres_host,
        CONFIG.postgres_port,
        CONFIG.postgres_db
    );

    PgPoolOptions::new()
        .max_connections(CONFIG.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(
            CONFIG.db_acquire_timeout_secs,
        ))
        .connect(&database_url)
        .await
        .unwrap()
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
