use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sqlx::postgres::PgPoolOptions;
use telegram_files_cache_server::views::get_router;
use tokio::sync::OnceCell;
use tower::ServiceExt;

/// Builds a Postgres pool that performs no network I/O at construction time
/// (`connect_lazy` only parses the URL; a real connection is opened lazily on
/// first query). None of the tests below issue a query, so this never touches
/// a network or a real database.
fn lazy_pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://test:test@localhost:1/test")
        .expect("connect_lazy only parses the URL, should not fail")
}

/// `CONFIG` is a process-wide `Lazy<Config>` that eagerly reads every required
/// env var on first access (from any test, in any order); populate fakes for
/// all of them so it doesn't panic. `get_router` also installs a *global*
/// metrics recorder (`axum_prometheus`) which can only be set once per
/// process, so the router itself is built exactly once and shared (cloned)
/// across all tests in this binary instead of being rebuilt per test.
static APP: OnceCell<axum::Router> = OnceCell::const_new();

async fn test_app() -> axum::Router {
    APP.get_or_init(|| async {
        for (k, v) in [
            ("API_KEY", "test-api-key"),
            ("POSTGRES_USER", "test"),
            ("POSTGRES_PASSWORD", "test"),
            ("POSTGRES_HOST", "localhost"),
            ("POSTGRES_PORT", "5432"),
            ("POSTGRES_DB", "test"),
            ("DOWNLOADER_API_KEY", "test"),
            ("DOWNLOADER_URL", "http://127.0.0.1:1"),
            ("LIBRARY_API_KEY", "test"),
            ("LIBRARY_URL", "http://127.0.0.1:1"),
            ("FILES_SERVER_API_KEY", "test"),
            ("FILES_SERVER_URL", "http://127.0.0.1:1"),
            ("BOT_TOKENS", "[]"),
            ("TEMP_CHANNEL_ID", "1"),
        ] {
            if std::env::var(k).is_err() {
                std::env::set_var(k, v);
            }
        }

        get_router(lazy_pool()).await
    })
    .await
    .clone()
}

#[tokio::test]
async fn missing_auth_header_returns_401() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/1/book/?copy=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_auth_header_returns_401() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/1/book/?copy=false")
                .header("Authorization", "definitely-wrong-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_check_returns_200_without_auth() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
