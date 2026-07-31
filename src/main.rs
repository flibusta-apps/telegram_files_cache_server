use dotenvy::dotenv;
use sentry::{integrations::debug_images::DebugImagesIntegration, types::Dsn, ClientOptions};
use sentry_tracing::EventFilter;
use std::{net::SocketAddr, str::FromStr};
use telegram_files_cache_server::{config, db, views};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{db::run_migrations, views::get_router};

#[tokio::main]
async fn main() {
    dotenv().ok();

    let dsn = config::CONFIG
        .sentry_dsn
        .as_ref()
        .map(|dsn| Dsn::from_str(dsn).expect("SENTRY_DSN must be a valid Sentry DSN"));

    let options = ClientOptions {
        dsn,
        default_integrations: false,
        ..Default::default()
    }
    .add_integration(DebugImagesIntegration::new());

    let _guard = sentry::init(options);

    let sentry_layer = sentry_tracing::layer()
        .event_filter(|md| match md.level() {
            &tracing::Level::ERROR => EventFilter::Event,
            _ => EventFilter::Ignore,
        })
        .enable_span_attributes();

    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(env_filter)
        .with(sentry_layer)
        .init();

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));

    // Get database pool
    let pool = db::get_pg_pool().await;

    // Run migrations
    info!("Running database migrations...");
    run_migrations(&pool)
        .await
        .expect("Failed to run database migrations");
    info!("Database migrations completed successfully");

    let app = get_router(pool).await;

    info!("Start webserver...");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind to 0.0.0.0:8080");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Webserver crashed");
    info!("Webserver shutdown...")
}

async fn shutdown_signal() {
    let sigterm = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    let sigint = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install SIGINT handler");
    };

    tokio::select! {
        _ = sigterm => {},
        _ = sigint => {},
    }

    info!("Shutdown signal received, starting graceful drain...");
}
