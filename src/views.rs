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
use tokio_util::io::ReaderStream;
use tower_http::trace::{self, TraceLayer};
use tracing::Level;

use crate::{
    config::CONFIG,
    db::get_pg_pool,
    serializers::CachedFile,
    services::{
        download_from_cache,
        download_utils::{build_download_headers, get_response_async_read},
        get_cached_file_copy, get_cached_file_or_cache, start_update_cache,
    },
};

pub type Database = PgPool;

//

#[derive(serde::Deserialize)]
pub struct GetCachedFileQuery {
    pub copy: bool,
    #[serde(default)]
    pub normalized: Option<bool>,
}

async fn get_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Query(GetCachedFileQuery { copy, normalized }): Query<GetCachedFileQuery>,
    Extension(Ext { db, .. }): Extension<Ext>,
) -> impl IntoResponse {
    let is_normalized = normalized.unwrap_or(true);
    let cached_file =
        match get_cached_file_or_cache(object_id, object_type, is_normalized, db.clone()).await {
            Ok(Some(cached_file)) => cached_file,
            Ok(None) => return StatusCode::NO_CONTENT.into_response(),
            Err(err) => {
                tracing::log::error!("{:?}", err);
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };

    if !copy {
        return Json(cached_file).into_response();
    }

    match get_cached_file_copy(cached_file, db).await {
        Ok(copy_file) => Json(copy_file).into_response(),
        Err(err) => {
            tracing::log::error!("{:?}", err);
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[derive(serde::Deserialize)]
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
                tracing::log::error!("{:?}", err);
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };

    let data = match download_from_cache(cached_file, db.clone()).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            let cached_file =
                match get_cached_file_or_cache(object_id, object_type, is_normalized, db.clone())
                    .await
                {
                    Ok(Some(v)) => v,
                    Ok(None) => return StatusCode::NO_CONTENT.into_response(),
                    Err(err) => {
                        tracing::log::error!("{:?}", err);
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                };

            match download_from_cache(cached_file, db).await {
                Ok(Some(v)) => v,
                Ok(None) => return StatusCode::NO_CONTENT.into_response(),
                Err(err) => {
                    tracing::log::error!("{:?}", err);
                    return StatusCode::BAD_GATEWAY.into_response();
                }
            }
        }
        Err(err) => {
            tracing::log::error!("{:?}", err);
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

#[derive(serde::Deserialize)]
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
    let cached_file: Option<CachedFile> = match sqlx::query_as!(
        CachedFile,
        r#"DELETE FROM cached_files
            WHERE object_id = $1 AND object_type = $2 AND is_normalized = $3
            RETURNING *"#,
        object_id,
        object_type,
        is_normalized
    )
    .fetch_optional(&db)
    .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::log::error!("{:?}", err);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    match cached_file {
        Some(v) => Json::<CachedFile>(v).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn update_cache(Extension(Ext { db, .. }): Extension<Ext>) -> impl IntoResponse {
    tokio::spawn(start_update_cache(db));

    StatusCode::OK.into_response()
}

async fn health_check() -> impl IntoResponse {
    StatusCode::OK.into_response()
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

    if auth_header != CONFIG.api_key {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

#[derive(Clone)]
struct Ext {
    pub db: PgPool,
}

pub async fn get_router() -> Router {
    let db = get_pg_pool().await;

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

    let metric_router =
        Router::new().route("/metrics", get(|| async move { metric_handle.render() }));

    let health_router = Router::new().route("/health", get(health_check));

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
