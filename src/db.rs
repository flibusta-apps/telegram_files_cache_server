use crate::config::CONFIG;

use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};

pub async fn get_pg_pool() -> PgPool {
    let options = PgConnectOptions::new()
        .host(&CONFIG.postgres_host)
        .port(CONFIG.postgres_port)
        .username(&CONFIG.postgres_user)
        .password(&CONFIG.postgres_password)
        .database(&CONFIG.postgres_db)
        .application_name("telegram_files_cache_server");

    PgPoolOptions::new()
        .max_connections(CONFIG.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(
            CONFIG.db_acquire_timeout_secs,
        ))
        .connect_with(options)
        .await
        .unwrap()
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
