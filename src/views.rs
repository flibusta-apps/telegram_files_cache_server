use axum::{
    body::Body,
    extract::{Path, Query},
    http::{self, Request, StatusCode},
    middleware::{self, Next},
    response::{AppendHeaders, IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use axum_prometheus::PrometheusMetricLayer;
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use tokio_util::io::ReaderStream;
use tower_http::trace::{self, TraceLayer};
use tracing::Level;

use crate::{
    config::CONFIG,
    repository::CachedFileRepository,
    serializers::CachedFile,
    services::{
        current_update_cache_status, delete_telegram_message, download_from_cache,
        download_utils::{build_download_headers, get_response_async_read},
        get_cached_file_copy, get_cached_file_or_cache, is_definitive_not_found_error,
        try_start_update_cache,
    },
};

pub type Database = PgPool;

//

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct GetCachedFileQuery {
    pub copy: bool,
    #[serde(default)]
    pub normalized: Option<bool>,
}

/// Returns cached-file metadata (`message_id`/`chat_id`) for `object_id`/`object_type`.
///
/// Staleness contract: with `copy=false` (the default) this returns the raw DB row
/// without validating that the referenced Telegram message still exists — it can be
/// stale if the message was deleted out-of-band (e.g. by admin action or a prior
/// eviction race). Consumers that need a validated reference should pass `?copy=true`
/// (validates and self-heals by re-caching on failure — see `get_cached_file_copy`) or
/// use `GET /download/...`, which validates on fetch and evicts+re-caches stale rows.
/// `created_at` on the returned row can help consumers decide how much to trust an old,
/// never-copied/never-downloaded entry.
async fn get_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Query(GetCachedFileQuery { copy, normalized }): Query<GetCachedFileQuery>,
    Extension(Ext { db, .. }): Extension<Ext>,
) -> impl IntoResponse {
    let is_normalized = normalized.unwrap_or(true);
    let object_type_for_log = object_type.clone();
    let cached_file =
        match get_cached_file_or_cache(object_id, object_type, is_normalized, db.clone()).await {
            Ok(Some(cached_file)) => cached_file,
            Ok(None) => return StatusCode::NO_CONTENT.into_response(),
            Err(err) => {
                tracing::error!(
                    object_id,
                    object_type = %object_type_for_log,
                    error = ?err,
                    "failed to get cached file"
                );
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };

    if !copy {
        return Json(cached_file).into_response();
    }

    let object_type_for_log = cached_file.object_type.clone();
    match get_cached_file_copy(cached_file, db).await {
        Ok(copy_file) => Json(copy_file).into_response(),
        Err(err) => {
            tracing::error!(
                object_id,
                object_type = %object_type_for_log,
                error = ?err,
                "failed to copy cached file"
            );
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct DownloadCachedFileQuery {
    #[serde(default)]
    pub normalized: Option<bool>,
}

async fn download_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Query(DownloadCachedFileQuery { normalized }): Query<DownloadCachedFileQuery>,
    Extension(Ext { db }): Extension<Ext>,
) -> impl IntoResponse {
    let is_normalized = normalized.unwrap_or(true);
    let cached_file =
        match get_cached_file_or_cache(object_id, object_type.clone(), is_normalized, db.clone())
            .await
        {
            Ok(Some(cached_file)) => cached_file,
            Ok(None) => return StatusCode::NO_CONTENT.into_response(),
            Err(err) => {
                tracing::error!(
                    object_id,
                    object_type = %object_type,
                    error = ?err,
                    "failed to get cached file"
                );
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };

    let data = match download_from_cache(cached_file, db.clone()).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let object_type_for_log = object_type.clone();
            let cached_file =
                match get_cached_file_or_cache(object_id, object_type, is_normalized, db.clone())
                    .await
                {
                    Ok(Some(v)) => v,
                    Ok(None) => return StatusCode::NO_CONTENT.into_response(),
                    Err(err) => {
                        tracing::error!(
                            object_id,
                            object_type = %object_type_for_log,
                            error = ?err,
                            "failed to get cached file on re-cache retry"
                        );
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                };

            match download_from_cache(cached_file, db).await {
                Ok(Some(v)) => v,
                Ok(None) => return StatusCode::NO_CONTENT.into_response(),
                Err(err) => {
                    tracing::error!(
                        object_id,
                        object_type = %object_type_for_log,
                        error = ?err,
                        "failed to download from cache on retry"
                    );
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        }
        Err(err) if is_definitive_not_found_error(&*err) => {
            // `download_from_cache` already evicted the stale row on this definitive
            // 404/410 before returning. Re-fetch (rebuilding the cache entry) and retry
            // once within this same request, mirroring the `Ok(None)` branch above —
            // otherwise this request gets a spurious 502 even though the very next
            // request would have rebuilt the entry successfully.
            tracing::warn!(
                object_id,
                object_type = %object_type,
                error = ?err,
                "cache entry was stale (definitive not-found); rebuilding and retrying"
            );

            let object_type_for_log = object_type.clone();
            let cached_file =
                match get_cached_file_or_cache(object_id, object_type, is_normalized, db.clone())
                    .await
                {
                    Ok(Some(v)) => v,
                    Ok(None) => return StatusCode::NO_CONTENT.into_response(),
                    Err(err) => {
                        tracing::error!(
                            object_id,
                            object_type = %object_type_for_log,
                            error = ?err,
                            "failed to get cached file on stale-entry retry"
                        );
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                };

            match download_from_cache(cached_file, db).await {
                Ok(Some(v)) => v,
                Ok(None) => return StatusCode::NO_CONTENT.into_response(),
                Err(err) => {
                    tracing::error!(
                        object_id,
                        object_type = %object_type_for_log,
                        error = ?err,
                        "failed to download from cache on stale-entry retry"
                    );
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        }
        Err(err) => {
            tracing::error!(
                object_id,
                object_type = %object_type,
                error = ?err,
                "failed to download from cache"
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let filename = data.filename.clone();
    let filename_ascii = data.filename_ascii.clone();
    let caption = data.caption.clone();

    let content_length = data.response.content_length();

    let reader = get_response_async_read(data.response);
    let stream = ReaderStream::new(reader);
    let body = Body::from_stream(stream);

    let headers = AppendHeaders(build_download_headers(
        &filename,
        &filename_ascii,
        &caption,
        content_length,
    ));

    (headers, body).into_response()
}

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct DeleteCachedFileQuery {
    #[serde(default)]
    pub normalized: Option<bool>,
}

async fn delete_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Query(DeleteCachedFileQuery { normalized }): Query<DeleteCachedFileQuery>,
    Extension(Ext { db, .. }): Extension<Ext>,
) -> impl IntoResponse {
    let is_normalized = normalized.unwrap_or(true);
    let object_type_for_log = object_type.clone();
    let cached_file: Option<CachedFile> = match CachedFileRepository::new(db)
        .delete_by_object_id_object_type_is_normalized(object_id, object_type, is_normalized)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(
                object_id,
                object_type = %object_type_for_log,
                error = ?err,
                "failed to delete cached file"
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    match cached_file {
        Some(v) => {
            delete_telegram_message(v.chat_id, v.message_id).await;
            Json::<CachedFile>(v).into_response()
        }
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn update_cache(Extension(Ext { db, .. }): Extension<Ext>) -> impl IntoResponse {
    match try_start_update_cache(db) {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => (StatusCode::CONFLICT, Json(current_update_cache_status())).into_response(),
    }
}

async fn health_check() -> impl IntoResponse {
    StatusCode::OK.into_response()
}

async fn readiness_check(Extension(Ext { db, .. }): Extension<Ext>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&db).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "readiness check DB probe failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

//

async fn auth(req: Request<axum::body::Body>, next: Next) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok());

    let auth_header = if let Some(auth_header) = auth_header {
        auth_header
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let provided = auth_header.as_bytes();
    let expected = CONFIG.api_key.as_bytes();
    if provided.len() != expected.len() || provided.ct_eq(expected).unwrap_u8() != 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

#[derive(Clone)]
struct Ext {
    pub db: PgPool,
}

pub async fn get_router(db: PgPool) -> Router {
    let ext = Ext { db };

    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let app_router = Router::new()
        .route("/{object_id}/{object_type}/", get(get_cached_file))
        .route(
            "/download/{object_id}/{object_type}/",
            get(download_cached_file),
        )
        .route("/{object_id}/{object_type}/", delete(delete_cached_file))
        .route("/update_cache", post(update_cache))
        .layer(middleware::from_fn(auth))
        .layer(Extension(ext.clone()))
        .layer(prometheus_layer);

    let metric_router = Router::new()
        .route(
            "/metrics",
            get(move || async move { metric_handle.render() }),
        )
        .layer(middleware::from_fn(auth));

    let health_router = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .layer(Extension(ext.clone()));

    Router::new()
        .nest("/api/v1/", app_router)
        .merge(metric_router)
        .merge(health_router)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(trace::DefaultMakeSpan::new().level(Level::INFO))
                .on_response(trace::DefaultOnResponse::new().level(Level::INFO)),
        )
}
