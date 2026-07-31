pub mod book_library;
pub mod bots;
pub mod download_utils;
pub mod downloader;
pub mod retry;
pub mod telegram_files;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use chrono::Duration;
use moka::future::Cache;
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::Serialize;
use teloxide::{
    requests::Requester,
    types::{ChatId, MessageId, Recipient},
};
use tracing::Instrument;

use crate::{config, repository::CachedFileRepository, serializers::CachedFile, views::Database};

use self::{
    book_library::{get_book, get_books, types::BaseBook},
    bots::ROUND_ROBIN_BOT,
    download_utils::DownloadResult,
    downloader::{download_from_downloader, get_filename, FilenameData},
    retry::{BACKGROUND_MAX_RETRIES, INTERACTIVE_MAX_RETRIES},
    telegram_files::{download_from_telegram_files, upload_to_telegram_files, UploadData},
};

#[derive(Serialize)]
pub struct CacheData {
    pub id: Option<i32>,
    pub object_id: i32,
    pub object_type: String,
    pub message_id: i32,
    pub chat_id: i64,
}

/// TTL for temp-channel copies created by `?copy=true`. This is part of the API
/// contract for that endpoint: consumers must forward/consume the returned message
/// within this window, after which this server deletes it from the temp channel.
const TEMP_MESSAGE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Tracks temp-channel copies produced by `?copy=true` so they can be cleaned up after
/// `TEMP_MESSAGE_TTL`. Keyed by the *temp message id* itself (unique per copy), not the
/// cached-file id — this way concurrent copies of the same cached file never collide and
/// evict/delete each other's still-in-use message.
pub static TEMP_MESSAGES: Lazy<Cache<i32, ()>> = Lazy::new(|| {
    Cache::builder()
        .time_to_live(TEMP_MESSAGE_TTL)
        .max_capacity(4098)
        .async_eviction_listener(|message_id, _value, _cause| {
            Box::pin(async move {
                let bot = ROUND_ROBIN_BOT.get_bot();
                if let Err(err) = bot
                    .delete_message(
                        Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
                        MessageId(*message_id),
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to delete expired temp message {} from temp channel: {:?}",
                        message_id,
                        err
                    );
                }
            })
        })
        .build()
});

/// Short-lived single-flight cache used to dedupe concurrent `cache_file` calls for the
/// same `(object_id, object_type, is_normalized)` key. This only guards against
/// duplicate concurrent Telegram uploads while a cache miss is being resolved; the
/// database row remains the real source of truth once the entry is written. Entries are
/// evicted quickly since we don't want to serve stale results from here.
type CacheFileKey = (i32, String, bool);
type CacheFileInflightCache = Cache<CacheFileKey, Arc<Option<CachedFile>>>;

static CACHE_FILE_INFLIGHT: Lazy<CacheFileInflightCache> = Lazy::new(|| {
    Cache::builder()
        .time_to_idle(std::time::Duration::from_secs(5))
        .max_capacity(256)
        .build()
});

/// Wraps a shared (`Arc`-ed) error produced by a deduped `try_get_with` call so it can be
/// propagated as a plain `Box<dyn Error + Send + Sync>`.
#[derive(Debug)]
struct SharedCacheError(Arc<Box<dyn std::error::Error + Send + Sync>>);

impl std::fmt::Display for SharedCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SharedCacheError {}

fn clone_cached_file(cached_file: &CachedFile) -> CachedFile {
    CachedFile {
        id: cached_file.id,
        object_id: cached_file.object_id,
        object_type: cached_file.object_type.clone(),
        is_normalized: cached_file.is_normalized,
        message_id: cached_file.message_id,
        chat_id: cached_file.chat_id,
        created_at: cached_file.created_at,
    }
}

/// Best-effort deletion of an uploaded Telegram message. This cache server owns every
/// message it uploads via `upload_to_telegram_files`, so once a cache row referencing a
/// message is removed (explicit DELETE, stale-entry eviction, or superseded by a
/// re-cache) the underlying Telegram message becomes orphaned unless we clean it up
/// here. Failures are logged, never propagated: losing this race must not fail the
/// caller's request.
pub async fn delete_telegram_message(chat_id: i64, message_id: i64) {
    let message_id: i32 = match message_id.try_into() {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                "Cannot delete orphaned Telegram message: message_id {} out of range: {:?}",
                message_id,
                err
            );
            return;
        }
    };

    let bot = ROUND_ROBIN_BOT.get_bot();
    if let Err(err) = bot
        .delete_message(Recipient::Id(ChatId(chat_id)), MessageId(message_id))
        .await
    {
        tracing::warn!(
            "Failed to delete orphaned Telegram message {} in chat {}: {:?}",
            message_id,
            chat_id,
            err
        );
    }
}

#[tracing::instrument(skip(db), fields(object_id = object_id, object_type = %object_type, is_normalized))]
pub async fn get_cached_file_or_cache(
    object_id: i32,
    object_type: String,
    is_normalized: bool,
    db: Database,
) -> Result<Option<CachedFile>, Box<dyn std::error::Error + Send + Sync>> {
    let cached_file = sqlx::query_as!(
        CachedFile,
        r#"
        SELECT * FROM cached_files
        WHERE object_id = $1 AND object_type = $2 AND is_normalized = $3"#,
        object_id,
        object_type,
        is_normalized
    )
    .fetch_optional(&db)
    .await?;

    match cached_file {
        Some(cached_file) => {
            metrics::counter!("cache_hits_total").increment(1);
            Ok(Some(cached_file))
        }
        None => {
            metrics::counter!("cache_misses_total").increment(1);

            let key = (object_id, object_type.clone(), is_normalized);

            match CACHE_FILE_INFLIGHT
                .try_get_with(key, async move {
                    cache_file(
                        object_id,
                        object_type,
                        is_normalized,
                        db,
                        INTERACTIVE_MAX_RETRIES,
                    )
                    .await
                    .map(Arc::new)
                })
                .await
            {
                Ok(shared) => Ok(shared.as_ref().as_ref().map(clone_cached_file)),
                Err(err) => Err(Box::new(SharedCacheError(err))),
            }
        }
    }
}

#[tracing::instrument(skip(db, original), fields(object_id = original.object_id, object_type = %original.object_type))]
pub async fn get_cached_file_copy(
    original: CachedFile,
    db: Database,
) -> Result<CacheData, Box<dyn std::error::Error + Send + Sync>> {
    let bot = ROUND_ROBIN_BOT.get_bot();

    let original_message_id: i32 = original.message_id.try_into().map_err(|err| {
        tracing::error!(
            "Invalid message_id {} for object_id {}: {:?}",
            original.message_id,
            original.object_id,
            err
        );
        Box::new(std::io::Error::other(format!(
            "invalid message_id {} for object_id {}",
            original.message_id, original.object_id
        ))) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let message_id = match bot
        .copy_message(
            Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
            Recipient::Id(ChatId(original.chat_id)),
            MessageId(original_message_id),
        )
        .await
    {
        Ok(v) => v,
        Err(_) => {
            sqlx::query!(
                r#"
                DELETE FROM cached_files
                WHERE id = $1
                "#,
                original.id
            )
            .execute(&db)
            .await?;

            delete_telegram_message(original.chat_id, original.message_id).await;

            let new_original = match get_cached_file_or_cache(
                original.object_id,
                original.object_type.clone(),
                original.is_normalized,
                db,
            )
            .await?
            {
                Some(v) => v,
                None => {
                    let err = std::io::Error::other(
                        "failed to re-cache file for copy: upstream returned no file",
                    );
                    tracing::error!("{:?}", err);
                    return Err(Box::new(err));
                }
            };

            let new_message_id: i32 = new_original.message_id.try_into().map_err(|err| {
                tracing::error!(
                    "Invalid message_id {} for object_id {}: {:?}",
                    new_original.message_id,
                    new_original.object_id,
                    err
                );
                Box::new(std::io::Error::other(format!(
                    "invalid message_id {} for object_id {}",
                    new_original.message_id, new_original.object_id
                ))) as Box<dyn std::error::Error + Send + Sync>
            })?;

            bot.copy_message(
                Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
                Recipient::Id(ChatId(new_original.chat_id)),
                MessageId(new_message_id),
            )
            .await?
        }
    };

    TEMP_MESSAGES.insert(message_id.0, ()).await;

    Ok(CacheData {
        id: None,
        object_id: original.object_id,
        object_type: original.object_type,
        message_id: message_id.0,
        chat_id: config::CONFIG.temp_channel_id,
    })
}

#[tracing::instrument(skip(db), fields(object_id = object_id, object_type = %object_type, is_normalized))]
pub async fn cache_file(
    object_id: i32,
    object_type: String,
    is_normalized: bool,
    db: Database,
    max_retries: u32,
) -> Result<Option<CachedFile>, Box<dyn std::error::Error + Send + Sync>> {
    let book = match get_book(object_id, max_retries).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let downloader_result = match download_from_downloader(
        book.source.id,
        book.remote_id,
        object_type.clone(),
        is_normalized,
        max_retries,
    )
    .await
    {
        Ok(v) => match v {
            Some(v) => v,
            None => return Ok(None),
        },
        Err(err) => {
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let UploadData {
        chat_id,
        message_id,
    } = match upload_to_telegram_files(downloader_result, book.get_caption(), max_retries).await {
        Ok(v) => v,
        Err(err) => {
            metrics::counter!("upload_failures_total").increment(1);
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let cached = sqlx::query_as!(
        CachedFile,
        r#"INSERT INTO cached_files (object_id, object_type, is_normalized, message_id, chat_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (object_id, object_type, is_normalized)
        DO UPDATE SET message_id = EXCLUDED.message_id, chat_id = EXCLUDED.chat_id
        RETURNING *"#,
        object_id,
        object_type,
        is_normalized,
        message_id,
        chat_id
    )
    .fetch_one(&db)
    .await?;

    Ok(Some(cached))
}

#[tracing::instrument(skip(db, cached_data), fields(object_id = cached_data.object_id, object_type = %cached_data.object_type))]
pub async fn download_from_cache(
    cached_data: CachedFile,
    db: Database,
) -> Result<Option<DownloadResult>, Box<dyn std::error::Error + Send + Sync>> {
    let response_task = tokio::task::spawn(
        download_from_telegram_files(
            cached_data.message_id,
            cached_data.chat_id,
            INTERACTIVE_MAX_RETRIES,
        )
        .instrument(tracing::Span::current()),
    );
    let filename_task = tokio::task::spawn(
        get_filename(
            cached_data.object_id,
            cached_data.object_type.clone(),
            cached_data.is_normalized,
            INTERACTIVE_MAX_RETRIES,
        )
        .instrument(tracing::Span::current()),
    );
    let book_task = tokio::task::spawn(
        get_book(cached_data.object_id, INTERACTIVE_MAX_RETRIES)
            .instrument(tracing::Span::current()),
    );

    let response = match response_task.await? {
        Ok(v) => match v.status() {
            StatusCode::OK => v,
            StatusCode::NO_CONTENT => {
                // Successful-but-empty response: not a "stale cache" signal, don't evict.
                return Ok(None);
            }
            other => {
                tracing::warn!(
                    "Unexpected non-error status {} from telegram_files download for object_id {}; using response as-is",
                    other,
                    cached_data.object_id
                );
                v
            }
        },
        Err(err) => {
            // download_from_telegram_files already applies error_for_status(), so any
            // status-based error here carries the real upstream status code. Only evict
            // the cache row on a definitive "message no longer exists" answer (404/410).
            // 5xx / timeouts / connect errors are transient and must not evict a valid cache entry.
            let status = err
                .downcast_ref::<reqwest::Error>()
                .and_then(|e| e.status());
            let should_evict = status.is_some_and(|status| {
                status == StatusCode::NOT_FOUND || status == StatusCode::GONE
            });

            if should_evict {
                let cached_file_repo = CachedFileRepository::new(db.clone());

                match cached_file_repo
                    .delete_by_object_id_object_type_is_normalized(
                        cached_data.object_id,
                        cached_data.object_type.clone(),
                        cached_data.is_normalized,
                    )
                    .await
                {
                    Ok(deleted) => {
                        delete_telegram_message(deleted.chat_id, deleted.message_id).await;
                        metrics::counter!(
                            "cache_evictions_total",
                            "reason" => if status == Some(StatusCode::NOT_FOUND) { "not_found" } else { "gone" }
                        )
                        .increment(1);
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Failed to evict stale cache row for object_id {}: {:?}",
                            cached_data.object_id,
                            err
                        );
                    }
                }
            } else {
                tracing::warn!(
                    "Non-definitive error fetching cached file for object_id {} (not evicting cache): {:?}",
                    cached_data.object_id,
                    err
                );
            }

            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let filename_data = match filename_task.await? {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let book = match book_task.await? {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let FilenameData {
        filename,
        filename_ascii,
    } = filename_data;
    let caption = book.get_caption();

    Ok(Some(DownloadResult {
        response,
        filename,
        filename_ascii,
        caption,
    }))
}

#[derive(Serialize)]
pub struct FileLinkResult {
    pub link: String,
    pub filename: String,
    pub filename_ascii: String,
    pub caption: String,
}

pub async fn get_books_for_update(
) -> Result<Vec<BaseBook>, Box<dyn std::error::Error + Send + Sync>> {
    let mut result: Vec<BaseBook> = vec![];

    let page_size = 50;

    let now = chrono::offset::Utc::now();
    let subset_3 = now - Duration::days(3);

    let uploaded_gte = subset_3.format("%Y-%m-%d").to_string();
    let uploaded_lte = now.format("%Y-%m-%d").to_string();

    let first_page = match get_books(
        1,
        page_size,
        uploaded_gte.clone(),
        uploaded_lte.clone(),
        BACKGROUND_MAX_RETRIES,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => return Err(err),
    };

    result.extend(first_page.items);

    let mut current_page = 2;
    let page_count = first_page.pages;

    while current_page <= page_count {
        let page = match get_books(
            current_page,
            page_size,
            uploaded_gte.clone(),
            uploaded_lte.clone(),
            BACKGROUND_MAX_RETRIES,
        )
        .await
        {
            Ok(v) => v,
            Err(err) => return Err(err),
        };
        result.extend(page.items);

        current_page += 1;
    }

    Ok(result)
}

#[derive(Default)]
struct UpdateCacheSummary {
    books_scanned: usize,
    files_cached: usize,
    failures: usize,
}

async fn start_update_cache(
    db: Database,
) -> Result<UpdateCacheSummary, Box<dyn std::error::Error + Send + Sync>> {
    let books = get_books_for_update().await?;

    let mut summary = UpdateCacheSummary::default();

    for book in books {
        summary.books_scanned += 1;

        'types: for available_type in book.available_types {
            let cached_file = match sqlx::query_as!(
                CachedFile,
                r#"SELECT * FROM cached_files
                   WHERE object_id = $1 AND object_type = $2 AND is_normalized = $3"#,
                book.id,
                available_type.clone(),
                true
            )
            .fetch_optional(&db)
            .await
            {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!("{:?}", err);
                    summary.failures += 1;
                    continue 'types;
                }
            };

            if cached_file.is_some() {
                continue 'types;
            }

            match cache_file(
                book.id,
                available_type,
                true,
                db.clone(),
                BACKGROUND_MAX_RETRIES,
            )
            .await
            {
                Ok(Some(_)) => {
                    summary.files_cached += 1;
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::error!("{:?}", err);
                    summary.failures += 1;
                }
            }
        }
    }

    Ok(summary)
}

static UPDATE_CACHE_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize)]
pub struct UpdateCacheStatus {
    pub running: bool,
    pub last_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
}

static UPDATE_CACHE_STATUS: Lazy<Mutex<UpdateCacheStatus>> = Lazy::new(|| {
    Mutex::new(UpdateCacheStatus {
        running: false,
        last_started_at: None,
        last_finished_at: None,
        last_error: None,
    })
});

pub fn current_update_cache_status() -> UpdateCacheStatus {
    UPDATE_CACHE_STATUS.lock().unwrap().clone()
}

/// Starts a cache-warmup scan unless one is already running. Returns `None` (and starts
/// nothing) if a scan is in flight, so repeated `POST /update_cache` calls coalesce onto
/// the single in-flight scan instead of racing on the same SELECT-then-INSERT per book
/// and causing duplicate-upload unique-constraint violations.
pub fn try_start_update_cache(db: Database) -> Option<UpdateCacheStatus> {
    if UPDATE_CACHE_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return None;
    }

    {
        let mut status = UPDATE_CACHE_STATUS.lock().unwrap();
        status.running = true;
        status.last_started_at = Some(chrono::Utc::now());
        status.last_error = None;
    }

    let run_span = tracing::info_span!("update_cache_run");

    tokio::spawn(
        async move {
            let started_at = std::time::Instant::now();
            let result = start_update_cache(db).await;
            let duration = started_at.elapsed();

            let outcome = if result.is_ok() { "success" } else { "failure" };
            metrics::counter!("update_cache_runs_total", "outcome" => outcome).increment(1);
            metrics::histogram!("update_cache_duration_seconds").record(duration.as_secs_f64());

            match &result {
                Ok(summary) => {
                    metrics::counter!("update_cache_cached_files_total")
                        .increment(summary.files_cached as u64);
                    tracing::info!(
                        books_scanned = summary.books_scanned,
                        files_cached = summary.files_cached,
                        failures = summary.failures,
                        duration_ms = duration.as_millis() as u64,
                        "update_cache run completed"
                    );
                }
                Err(err) => {
                    tracing::error!("update_cache scan failed: {:?}", err);
                    tracing::error!(
                        duration_ms = duration.as_millis() as u64,
                        "update_cache run failed"
                    );
                }
            }

            let mut status = UPDATE_CACHE_STATUS.lock().unwrap();
            status.running = false;
            status.last_finished_at = Some(chrono::Utc::now());
            status.last_error = result.err().map(|e| e.to_string());
            drop(status);

            UPDATE_CACHE_RUNNING.store(false, Ordering::SeqCst);
        }
        .instrument(run_span),
    );

    Some(current_update_cache_status())
}
