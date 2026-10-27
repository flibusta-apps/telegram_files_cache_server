pub mod book_library;
pub mod bots;
pub mod download_utils;
pub mod downloader;
pub mod retry;
pub mod reuse;
pub mod telegram_files;
pub mod warmup;
pub mod zip_utils;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use chrono::Duration;
use futures::future::{BoxFuture, FutureExt, Shared};
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::Serialize;
use teloxide::{
    requests::Requester,
    types::{ChatId, MessageId, Recipient},
    ApiError, RequestError,
};
use tracing::Instrument;

use crate::{
    config,
    error::{rate_limit_secs, BoxError, SharedError},
    repository::CachedFileRepository,
    serializers::CachedFile,
    views::Database,
};

use self::{
    book_library::{get_book, get_books},
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

/// Schedules deletion of a temp-channel copy after `TEMP_MESSAGE_TTL`. Each copy gets its
/// own sleeping task, so (unlike a capacity-bounded cache) live copies are never deleted
/// early because of eviction pressure.
fn schedule_temp_message_deletion(message_id: i32) {
    tokio::spawn(async move {
        tokio::time::sleep(TEMP_MESSAGE_TTL).await;
        let bot = ROUND_ROBIN_BOT.get_bot();
        if let Err(err) = bot
            .delete_message(
                Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
                MessageId(message_id),
            )
            .await
        {
            tracing::warn!(
                "Failed to delete expired temp message {} from temp channel: {:?}",
                message_id,
                err
            );
        }
    });
}

/// Single-flight registry: dedupes only *in-flight* `cache_file` calls for the same
/// `(object_id, object_type, is_normalized)` key. The entry removes itself the moment the
/// operation finishes, so finished results are never served from here — the database row
/// is the source of truth.
type CacheFileKey = (i32, String, bool);
type FlightResult = Result<Option<CachedFile>, SharedError>;
type Flight = Shared<BoxFuture<'static, FlightResult>>;

static CACHE_FILE_INFLIGHT: Lazy<Mutex<HashMap<CacheFileKey, Flight>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn lock_inflight() -> std::sync::MutexGuard<'static, HashMap<CacheFileKey, Flight>> {
    CACHE_FILE_INFLIGHT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Runs `cache_file` through the single-flight registry. Used both by request handlers
/// and by the warmup job so they never upload the same file concurrently.
pub async fn cache_file_single_flight(
    object_id: i32,
    object_type: String,
    is_normalized: bool,
    db: Database,
    max_retries: u32,
) -> Result<Option<CachedFile>, BoxError> {
    let key: CacheFileKey = (object_id, object_type.clone(), is_normalized);

    let flight = {
        let mut map = lock_inflight();
        if let Some(existing) = map.get(&key) {
            existing.clone()
        } else {
            let key_for_cleanup = key.clone();
            let fut: BoxFuture<'static, FlightResult> = async move {
                let result =
                    cache_file(object_id, object_type, is_normalized, db, max_retries).await;
                lock_inflight().remove(&key_for_cleanup);
                result.map_err(|e| SharedError(Arc::new(e)))
            }
            .boxed();
            let shared = fut.shared();
            map.insert(key, shared.clone());
            shared
        }
    };

    flight.await.map_err(|e| Box::new(e) as BoxError)
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

/// Removes exactly the row version that was read (same id/chat_id/message_id). Only when a
/// row was actually deleted (and `delete_telegram` is set) is the Telegram message cleaned
/// up, so a concurrently re-cached row (new message) is never touched.
async fn evict_exact(db: &Database, row: &CachedFile, delete_telegram: bool, reason: &'static str) {
    match CachedFileRepository::new(db.clone())
        .delete_exact(row)
        .await
    {
        Ok(true) => {
            if delete_telegram {
                delete_telegram_message(row.chat_id, row.message_id).await;
            }
            metrics::counter!("cache_evictions_total", "reason" => reason).increment(1);
        }
        Ok(false) => {
            tracing::debug!(
                "No cache row to evict for object_id {} (already replaced or gone)",
                row.object_id
            );
        }
        Err(err) => {
            tracing::warn!(
                "Failed to evict stale cache row for object_id {}: {:?}",
                row.object_id,
                err
            );
        }
    }
}

/// Whether a failed `copy_message` means the source message is definitively gone (so the
/// cache row is stale). Rate limits, network errors, timeouts, rights problems etc. are
/// not: those must be surfaced without deleting anything.
pub fn is_copy_source_gone(err: &RequestError) -> bool {
    matches!(
        err,
        RequestError::Api(ApiError::MessageToCopyNotFound | ApiError::MessageIdInvalid)
    )
}

#[tracing::instrument(skip(db), fields(object_id = object_id, object_type = %object_type, is_normalized))]
pub async fn get_cached_file_or_cache(
    object_id: i32,
    object_type: String,
    is_normalized: bool,
    db: Database,
) -> Result<Option<CachedFile>, BoxError> {
    let cached_file = CachedFileRepository::new(db.clone())
        .find_by_object_id_object_type_is_normalized(object_id, object_type.clone(), is_normalized)
        .await?;

    match cached_file {
        Some(cached_file) => {
            metrics::counter!("cache_hits_total").increment(1);
            Ok(Some(cached_file))
        }
        None => {
            metrics::counter!("cache_misses_total").increment(1);
            cache_file_single_flight(
                object_id,
                object_type,
                is_normalized,
                db,
                INTERACTIVE_MAX_RETRIES,
            )
            .await
        }
    }
}

fn invalid_message_id(file: &CachedFile) -> BoxError {
    tracing::error!(
        "Invalid message_id {} for object_id {}",
        file.message_id,
        file.object_id
    );
    Box::new(std::io::Error::other(format!(
        "invalid message_id {} for object_id {}",
        file.message_id, file.object_id
    )))
}

#[tracing::instrument(skip(db, original), fields(object_id = original.object_id, object_type = %original.object_type))]
pub async fn get_cached_file_copy(
    original: CachedFile,
    db: Database,
) -> Result<CacheData, BoxError> {
    let bot = ROUND_ROBIN_BOT.get_bot();

    let original_message_id: i32 = original
        .message_id
        .try_into()
        .map_err(|_| invalid_message_id(&original))?;

    let message_id = match bot
        .copy_message(
            Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
            Recipient::Id(ChatId(original.chat_id)),
            MessageId(original_message_id),
        )
        .await
    {
        Ok(v) => v,
        Err(err) if is_copy_source_gone(&err) => {
            tracing::warn!(
                object_id = original.object_id,
                "cached Telegram message is gone ({:?}); evicting and re-caching",
                err
            );
            evict_exact(&db, &original, true, "copy_source_gone").await;

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

            let new_message_id: i32 = new_original
                .message_id
                .try_into()
                .map_err(|_| invalid_message_id(&new_original))?;

            bot.copy_message(
                Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
                Recipient::Id(ChatId(new_original.chat_id)),
                MessageId(new_message_id),
            )
            .await?
        }
        Err(err) => {
            tracing::error!(
                object_id = original.object_id,
                "copy_message failed (not evicting cache): {:?}",
                err
            );
            return Err(Box::new(err));
        }
    };

    schedule_temp_message_deletion(message_id.0);

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
) -> Result<Option<CachedFile>, BoxError> {
    let book = match get_book(object_id, max_retries).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let source_id = book.source.id;
    let remote_id = book.remote_id;
    let caption = book.get_caption();

    let upload_result = match reuse::try_reuse_from_opposite_normalized(
        object_id,
        &object_type,
        is_normalized,
        &db,
        &caption,
        max_retries,
    )
    .await?
    {
        reuse::ReuseOutcome::Uploaded(v) => v,
        reuse::ReuseOutcome::NotEligible => {
            let downloader_result = match download_from_downloader(
                source_id,
                remote_id,
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

            match upload_to_telegram_files(downloader_result, caption, max_retries).await {
                Ok(v) => v,
                Err(err) => {
                    metrics::counter!("upload_failures_total").increment(1);
                    tracing::error!("{:?}", err);
                    return Err(err);
                }
            }
        }
    };

    let UploadData {
        chat_id,
        message_id,
    } = upload_result;

    // DO NOTHING on conflict: if a concurrent writer already cached this key, keep its row
    // and delete our freshly uploaded duplicate instead of orphaning/overwriting messages.
    let inserted = sqlx::query_as!(
        CachedFile,
        r#"INSERT INTO cached_files (object_id, object_type, is_normalized, message_id, chat_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (object_id, object_type, is_normalized) DO NOTHING
        RETURNING id, object_id, object_type, is_normalized, message_id, chat_id, created_at"#,
        object_id,
        object_type,
        is_normalized,
        message_id,
        chat_id
    )
    .fetch_optional(&db)
    .await;

    match inserted {
        Ok(Some(cached)) => Ok(Some(cached)),
        Ok(None) => {
            delete_telegram_message(chat_id, message_id).await;
            let existing = CachedFileRepository::new(db)
                .find_by_object_id_object_type_is_normalized(object_id, object_type, is_normalized)
                .await?;
            Ok(existing)
        }
        Err(err) => {
            // Row was not stored, so the uploaded message is unreferenced.
            delete_telegram_message(chat_id, message_id).await;
            Err(Box::new(err))
        }
    }
}

/// What `download_from_cache` should do with the cache row, based on the upstream status
/// of the files server answer (C1 contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictDecision {
    /// Do not touch the row (success, 429, 5xx, timeouts, other errors).
    Keep,
    /// Message is definitively gone (410): delete the row only.
    DropRow,
    /// Message exists but has no document (204) or legacy 404: delete the stale row, and
    /// the Telegram message too where it may still exist.
    DropRowAndMessage,
}

/// Decides eviction from an upstream HTTP status, per contract C1: evict on 410 only
/// (plus 404 from older files servers); 204 drops the stale row; never on 429/5xx.
pub fn evict_decision(status: Option<StatusCode>) -> EvictDecision {
    match status {
        Some(StatusCode::GONE) | Some(StatusCode::NOT_FOUND) => EvictDecision::DropRow,
        Some(StatusCode::NO_CONTENT) => EvictDecision::DropRowAndMessage,
        _ => EvictDecision::Keep,
    }
}

/// A definitive "the referenced Telegram message no longer exists" signal (404/410).
/// Used by `views::download_cached_file` to decide whether to retry-rebuild within the
/// same request.
pub fn is_definitive_not_found_status(status: Option<StatusCode>) -> bool {
    status.is_some_and(|status| status == StatusCode::NOT_FOUND || status == StatusCode::GONE)
}

/// Extracts the upstream HTTP status from a boxed error, if it carries one.
pub fn is_definitive_not_found_error(
    err: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    let status = err
        .downcast_ref::<reqwest::Error>()
        .and_then(|e| e.status());
    is_definitive_not_found_status(status)
}

#[tracing::instrument(skip(db, cached_data), fields(object_id = cached_data.object_id, object_type = %cached_data.object_type))]
pub async fn download_from_cache(
    cached_data: CachedFile,
    db: Database,
) -> Result<Option<DownloadResult>, BoxError> {
    let (response_result, filename_result, book_result) = tokio::join!(
        download_from_telegram_files(
            cached_data.message_id,
            cached_data.chat_id,
            INTERACTIVE_MAX_RETRIES,
        ),
        get_filename(
            cached_data.object_id,
            cached_data.object_type.clone(),
            cached_data.is_normalized,
            INTERACTIVE_MAX_RETRIES,
        ),
        get_book(cached_data.object_id, INTERACTIVE_MAX_RETRIES),
    );

    let response = match response_result {
        Ok(v) => match v.status() {
            StatusCode::OK => v,
            StatusCode::NO_CONTENT => {
                // Message exists but carries no document: the row is useless. Drop it so
                // the next request re-caches, but do not loop within this request.
                tracing::warn!(
                    object_id = cached_data.object_id,
                    "cached message has no document; dropping stale row"
                );
                evict_exact(&db, &cached_data, true, "no_document").await;
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
            let status = err
                .downcast_ref::<reqwest::Error>()
                .and_then(|e| e.status());
            match evict_decision(status) {
                EvictDecision::DropRow | EvictDecision::DropRowAndMessage => {
                    // The message is already gone upstream: only the row is deleted.
                    let reason = if status == Some(StatusCode::NOT_FOUND) {
                        "not_found"
                    } else {
                        "gone"
                    };
                    evict_exact(&db, &cached_data, false, reason).await;
                }
                EvictDecision::Keep => {
                    tracing::warn!(
                        "Non-definitive error fetching cached file for object_id {} (not evicting cache): {:?}",
                        cached_data.object_id,
                        err
                    );
                }
            }

            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let filename_data = match filename_result {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("{:?}", err);
            return Err(err);
        }
    };

    let book = match book_result {
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

#[derive(Default)]
struct UpdateCacheSummary {
    books_scanned: usize,
    files_cached: usize,
    failures: usize,
}

/// One `(book_id, object_type)` pair to be cache-warmed. `is_normalized` is always
/// `true` for warmup (matches the prior sequential behavior).
type WarmupPair = (i32, String);

/// Caches a single `(book_id, object_type)` pair unless it's already cached. Returns
/// `true` on success (including "already cached, nothing to do"), `false` on failure.
/// Increments `files_cached` (shared across the page's workers) only when a *new* file
/// was actually cached, so `UpdateCacheSummary::files_cached` keeps its original
/// meaning even though `run_with_concurrency` only tracks success/failure. On a 429
/// from the files server, extends the shared `throttle` backoff before reporting
/// failure so sibling in-flight workers pause too.
async fn warm_up_pair(
    (book_id, object_type): WarmupPair,
    db: Database,
    throttle: Arc<warmup::WarmupThrottle>,
    files_cached: Arc<std::sync::atomic::AtomicUsize>,
) -> bool {
    let cached_file = match CachedFileRepository::new(db.clone())
        .find_by_object_id_object_type_is_normalized(book_id, object_type.clone(), true)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::error!("{:?}", err);
            return false;
        }
    };

    if cached_file.is_some() {
        return true;
    }

    match cache_file_single_flight(book_id, object_type, true, db, BACKGROUND_MAX_RETRIES).await {
        Ok(Some(_)) => {
            files_cached.fetch_add(1, Ordering::Relaxed);
            true
        }
        Ok(None) => true,
        Err(err) => {
            if let Some(secs) = rate_limit_secs(&*err) {
                let backoff = warmup::WARMUP_429_BACKOFF.max(std::time::Duration::from_secs(secs));
                throttle.trigger_backoff(backoff).await;
            }

            tracing::error!("{:?}", err);
            false
        }
    }
}

async fn start_update_cache(db: Database) -> Result<UpdateCacheSummary, BoxError> {
    let page_size = 50;

    let now = chrono::offset::Utc::now();
    let subset_3 = now - Duration::days(3);

    let uploaded_gte = subset_3.format("%Y-%m-%d").to_string();
    let uploaded_lte = now.format("%Y-%m-%d").to_string();

    let mut summary = UpdateCacheSummary::default();
    let throttle = Arc::new(warmup::WarmupThrottle::new());
    let concurrency = config::CONFIG.cache_warmup_concurrency;

    let first_page = get_books(
        1,
        page_size,
        uploaded_gte.clone(),
        uploaded_lte.clone(),
        BACKGROUND_MAX_RETRIES,
    )
    .await?;
    let page_count = first_page.pages;

    let mut current_page_books = first_page.items;
    let mut next_page_number = 2;

    loop {
        // Kick off the fetch for the next page (if any) before processing the current
        // one, so network fetch and cache processing overlap.
        let next_page_fetch = if next_page_number <= page_count {
            let uploaded_gte = uploaded_gte.clone();
            let uploaded_lte = uploaded_lte.clone();
            let page_number = next_page_number;
            Some(tokio::spawn(async move {
                get_books(
                    page_number,
                    page_size,
                    uploaded_gte,
                    uploaded_lte,
                    BACKGROUND_MAX_RETRIES,
                )
                .await
            }))
        } else {
            None
        };

        summary.books_scanned += current_page_books.len();

        let pairs: Vec<WarmupPair> = current_page_books
            .into_iter()
            .flat_map(|book| {
                book.available_types
                    .into_iter()
                    .map(move |object_type| (book.id, object_type))
            })
            .collect();

        let db_for_workers = db.clone();
        let files_cached = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let files_cached_for_workers = files_cached.clone();
        let (_successes, failures) = warmup::run_with_concurrency(
            pairs,
            concurrency,
            throttle.clone(),
            move |pair, throttle| {
                let db = db_for_workers.clone();
                let files_cached = files_cached_for_workers.clone();
                async move { warm_up_pair(pair, db, throttle, files_cached).await }
            },
        )
        .await;

        summary.files_cached += files_cached.load(Ordering::Relaxed);
        summary.failures += failures;

        let Some(next_page_fetch) = next_page_fetch else {
            break;
        };

        let next_page = next_page_fetch.await??;
        current_page_books = next_page.items;
        next_page_number += 1;
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

fn lock_status() -> std::sync::MutexGuard<'static, UpdateCacheStatus> {
    UPDATE_CACHE_STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Resets `UPDATE_CACHE_RUNNING` (and the status `running` flag) on drop, including when
/// the scan task panics.
struct UpdateCacheGuard;

impl Drop for UpdateCacheGuard {
    fn drop(&mut self) {
        let mut status = lock_status();
        status.running = false;
        status.last_finished_at = Some(chrono::Utc::now());
        drop(status);
        UPDATE_CACHE_RUNNING.store(false, Ordering::SeqCst);
    }
}

pub fn current_update_cache_status() -> UpdateCacheStatus {
    lock_status().clone()
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
        let mut status = lock_status();
        status.running = true;
        status.last_started_at = Some(chrono::Utc::now());
        status.last_error = None;
    }

    let run_span = tracing::info_span!("update_cache_run");

    tokio::spawn(
        async move {
            let _guard = UpdateCacheGuard;
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

            lock_status().last_error = result.err().map(|e| e.to_string());
        }
        .instrument(run_span),
    );

    Some(current_update_cache_status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evict_decision_by_status() {
        assert_eq!(
            evict_decision(Some(StatusCode::GONE)),
            EvictDecision::DropRow
        );
        assert_eq!(
            evict_decision(Some(StatusCode::NOT_FOUND)),
            EvictDecision::DropRow
        );
        assert_eq!(
            evict_decision(Some(StatusCode::NO_CONTENT)),
            EvictDecision::DropRowAndMessage
        );
        for s in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::OK,
        ] {
            assert_eq!(evict_decision(Some(s)), EvictDecision::Keep, "{s}");
        }
        assert_eq!(evict_decision(None), EvictDecision::Keep);
    }

    #[test]
    fn copy_source_gone_only_for_definitive_api_errors() {
        assert!(is_copy_source_gone(&RequestError::Api(
            ApiError::MessageToCopyNotFound
        )));
        assert!(is_copy_source_gone(&RequestError::Api(
            ApiError::MessageIdInvalid
        )));
        assert!(!is_copy_source_gone(&RequestError::Api(
            ApiError::BotBlocked
        )));
        assert!(!is_copy_source_gone(&RequestError::Io(
            std::io::Error::other("net").into()
        )));
    }
}
