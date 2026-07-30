pub mod book_library;
pub mod bots;
pub mod download_utils;
pub mod downloader;
pub mod retry;
pub mod telegram_files;

use chrono::Duration;
use moka::future::Cache;
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::Serialize;
use teloxide::{
    requests::Requester,
    types::{ChatId, MessageId, Recipient},
};
use tracing::log;

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

pub static TEMP_MESSAGES: Lazy<Cache<i32, MessageId>> = Lazy::new(|| {
    Cache::builder()
        .time_to_idle(std::time::Duration::from_secs(16))
        .max_capacity(4098)
        .async_eviction_listener(|_data_id, message_id, _cause| {
            Box::pin(async move {
                let bot = ROUND_ROBIN_BOT.get_bot();
                let _ = bot
                    .delete_message(
                        Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
                        message_id,
                    )
                    .await;
            })
        })
        .build()
});

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
        Some(cached_file) => Ok(Some(cached_file)),
        None => {
            cache_file(
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

pub async fn get_cached_file_copy(
    original: CachedFile,
    db: Database,
) -> Result<CacheData, Box<dyn std::error::Error + Send + Sync>> {
    let bot = ROUND_ROBIN_BOT.get_bot();

    let original_message_id: i32 = original.message_id.try_into().map_err(|err| {
        log::error!(
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
                    log::error!("{:?}", err);
                    return Err(Box::new(err));
                }
            };

            let new_message_id: i32 = new_original.message_id.try_into().map_err(|err| {
                log::error!(
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

    TEMP_MESSAGES.insert(original.id, message_id).await;

    Ok(CacheData {
        id: None,
        object_id: original.object_id,
        object_type: original.object_type,
        message_id: message_id.0,
        chat_id: config::CONFIG.temp_channel_id,
    })
}

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
            log::error!("{:?}", err);
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
            log::error!("{:?}", err);
            return Err(err);
        }
    };

    let UploadData {
        chat_id,
        message_id,
    } = match upload_to_telegram_files(downloader_result, book.get_caption(), max_retries).await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return Err(err);
        }
    };

    let cached = sqlx::query_as!(
        CachedFile,
        r#"INSERT INTO cached_files (object_id, object_type, is_normalized, message_id, chat_id)
        VALUES ($1, $2, $3, $4, $5)
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

pub async fn download_from_cache(
    cached_data: CachedFile,
    db: Database,
) -> Result<Option<DownloadResult>, Box<dyn std::error::Error + Send + Sync>> {
    let response_task = tokio::task::spawn(download_from_telegram_files(
        cached_data.message_id,
        cached_data.chat_id,
        INTERACTIVE_MAX_RETRIES,
    ));
    let filename_task = tokio::task::spawn(get_filename(
        cached_data.object_id,
        cached_data.object_type.clone(),
        cached_data.is_normalized,
        INTERACTIVE_MAX_RETRIES,
    ));
    let book_task = tokio::task::spawn(get_book(cached_data.object_id, INTERACTIVE_MAX_RETRIES));

    let response = match response_task.await? {
        Ok(v) => match v.status() {
            StatusCode::OK => v,
            StatusCode::NO_CONTENT => {
                // Successful-but-empty response: not a "stale cache" signal, don't evict.
                return Ok(None);
            }
            other => {
                log::warn!(
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
            let should_evict = err
                .downcast_ref::<reqwest::Error>()
                .and_then(|e| e.status())
                .is_some_and(|status| {
                    status == StatusCode::NOT_FOUND || status == StatusCode::GONE
                });

            if should_evict {
                let cached_file_repo = CachedFileRepository::new(db.clone());

                let _ = cached_file_repo
                    .delete_by_object_id_object_type_is_normalized(
                        cached_data.object_id,
                        cached_data.object_type.clone(),
                        cached_data.is_normalized,
                    )
                    .await;
            } else {
                log::warn!(
                    "Non-definitive error fetching cached file for object_id {} (not evicting cache): {:?}",
                    cached_data.object_id,
                    err
                );
            }

            log::error!("{:?}", err);
            return Err(err);
        }
    };

    let filename_data = match filename_task.await? {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return Err(err);
        }
    };

    let book = match book_task.await? {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
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

pub async fn start_update_cache(db: Database) {
    let books = match get_books_for_update().await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return;
        }
    };

    for book in books {
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
                    log::error!("{:?}", err);
                    continue 'types;
                }
            };

            if cached_file.is_some() {
                continue 'types;
            }

            if let Err(err) = cache_file(
                book.id,
                available_type,
                true,
                db.clone(),
                BACKGROUND_MAX_RETRIES,
            )
            .await
            {
                log::error!("{:?}", err);
            }
        }
    }
}
