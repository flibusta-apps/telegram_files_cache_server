use axum::{Router, response::{Response, IntoResponse, AppendHeaders}, http::{StatusCode, self, Request, header}, middleware::{Next, self}, Extension, routing::{get, delete, post}, extract::Path, Json, body::StreamBody};
use axum_prometheus::PrometheusMetricLayer;
use tokio_util::io::ReaderStream;
use tower_http::trace::{TraceLayer, self};
use tracing::{Level, log};
use std::sync::Arc;
use base64::{engine::general_purpose, Engine};

use crate::{config::CONFIG, db::get_prisma_client, prisma::{PrismaClient, cached_file::{self}}, services::{get_cached_file_or_cache, download_from_cache, download_utils::get_response_async_read, start_update_cache, get_download_link}};


pub type Database = Arc<PrismaClient>;

//

async fn get_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Extension(Ext { db, .. }): Extension<Ext>
) -> impl IntoResponse {
    match get_cached_file_or_cache(object_id, object_type, db).await {
        Some(cached_file) => Json(cached_file).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn download_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Extension(Ext { db }): Extension<Ext>
) -> impl IntoResponse {
    let cached_file = match get_cached_file_or_cache(object_id, object_type.clone(), db.clone()).await {
        Some(cached_file) => cached_file,
        None => return StatusCode::NO_CONTENT.into_response(),
    };

    let data = match download_from_cache(cached_file, db.clone()).await {
        Some(v) => v,
        None => {
            let cached_file = match get_cached_file_or_cache(object_id, object_type, db.clone()).await {
                Some(v) => v,
                None => return StatusCode::NO_CONTENT.into_response(),
            };

            match download_from_cache(cached_file, db).await {
                Some(v) => v,
                None => return StatusCode::NO_CONTENT.into_response(),
            }
        }
    };

    let filename = data.filename.clone();
    let filename_ascii = data.filename_ascii.clone();
    let caption = data.caption.clone();

    let encoder = general_purpose::STANDARD;

    let reader = get_response_async_read(data.response);
    let stream = ReaderStream::new(reader);
    let body = StreamBody::new(stream);

    let headers = AppendHeaders([
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename={filename_ascii}"),
        ),
        (
            header::HeaderName::from_static("x-filename-b64"),
            encoder.encode(filename),
        ),
        (
            header::HeaderName::from_static("x-caption-b64"),
            encoder.encode(caption)
        )
    ]);

    (headers, body).into_response()
}

async fn get_link(
    Path((object_id, object_type)): Path<(i32, String)>,
    Extension(Ext { db, .. }): Extension<Ext>
) -> impl IntoResponse {
    match get_download_link(object_id, object_type.clone(), db.clone()).await {
        Ok(data) => {
            match data {
                Some(data) => Json(data).into_response(),
                None => return StatusCode::NO_CONTENT.into_response(),
            }
        },
        Err(err) => {
            log::error!("{:?}", err);
            return StatusCode::NO_CONTENT.into_response();
        },
    }
}

async fn delete_cached_file(
    Path((object_id, object_type)): Path<(i32, String)>,
    Extension(Ext { db, .. }): Extension<Ext>
) -> impl IntoResponse {
    let cached_file = db.cached_file()
        .find_unique(cached_file::object_id_object_type(object_id, object_type.clone()))
        .exec()
        .await
        .unwrap();

    match cached_file {
        Some(v) => {
            db.cached_file()
                .delete(cached_file::object_id_object_type(object_id, object_type))
                .exec()
                .await
                .unwrap();

            Json(v).into_response()
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn update_cache(
    Extension(Ext { db, .. }): Extension<Ext>
) -> impl IntoResponse {
    tokio::spawn(start_update_cache(db));

    StatusCode::OK.into_response()
}

//


async fn auth<B>(req: Request<B>, next: Next<B>) -> Result<Response, StatusCode> {
    let auth_header = req.headers()
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
    pub db: Arc<PrismaClient>,
}


pub async fn get_router() -> Router {
    let db = Arc::new(get_prisma_client().await);

    let ext = Ext { db };

    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let app_router = Router::new()
        .route("/:object_id/:object_type/", get(get_cached_file))
        .route("/download/:object_id/:object_type/", get(download_cached_file))
        .route("/link/:object_id/:object_type/", get(get_link))
        .route("/:object_id/:object_type/", delete(delete_cached_file))
        .route("/update_cache", post(update_cache))

        .layer(middleware::from_fn(auth))
        .layer(Extension(ext))
        .layer(prometheus_layer);

    let metric_router = Router::new()
        .route("/metrics", get(|| async move { metric_handle.render() }));

    Router::new()
        .nest("/api/v1/", app_router)
        .nest("/", metric_router)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(trace::DefaultMakeSpan::new()
                    .level(Level::INFO))
                .on_response(trace::DefaultOnResponse::new()
                    .level(Level::INFO)),
        )
}
