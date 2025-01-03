pub mod book_library;
pub mod bots;
pub mod download_utils;
pub mod downloader;
pub mod telegram_files;

use chrono::Duration;
use moka::future::Cache;
use once_cell::sync::Lazy;
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
    db: Database,
) -> Option<CachedFile> {
    let cached_file = sqlx::query_as!(
        CachedFile,
        r#"
        SELECT * FROM cached_files
        WHERE object_id = $1 AND object_type = $2"#,
        object_id,
        object_type
    )
    .fetch_optional(&db)
    .await
    .unwrap();

    match cached_file {
        Some(cached_file) => Some(cached_file),
        None => cache_file(object_id, object_type, db).await,
    }
}

pub async fn get_cached_file_copy(original: CachedFile, db: Database) -> CacheData {
    let bot = ROUND_ROBIN_BOT.get_bot();

    let message_id = match bot
        .copy_message(
            Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
            Recipient::Id(ChatId(original.chat_id)),
            MessageId(original.message_id.try_into().unwrap()),
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
            .await
            .unwrap();

            let new_original =
                get_cached_file_or_cache(original.object_id, original.object_type.clone(), db)
                    .await
                    .unwrap();

            bot.copy_message(
                Recipient::Id(ChatId(config::CONFIG.temp_channel_id)),
                Recipient::Id(ChatId(new_original.chat_id)),
                MessageId(new_original.message_id.try_into().unwrap()),
            )
            .await
            .unwrap()
        }
    };

    TEMP_MESSAGES.insert(original.id, message_id).await;

    CacheData {
        id: None,
        object_id: original.object_id,
        object_type: original.object_type,
        message_id: message_id.0,
        chat_id: config::CONFIG.temp_channel_id,
    }
}

pub async fn cache_file(object_id: i32, object_type: String, db: Database) -> Option<CachedFile> {
    let book = match get_book(object_id).await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        }
    };

    let downloader_result =
        match download_from_downloader(book.source.id, book.remote_id, object_type.clone()).await {
            Ok(v) => match v {
                Some(v) => v,
                None => return None,
            },
            Err(err) => {
                log::error!("{:?}", err);
                return None;
            }
        };

    let UploadData {
        chat_id,
        message_id,
    } = match upload_to_telegram_files(downloader_result, book.get_caption()).await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        }
    };

    Some(
        sqlx::query_as!(
            CachedFile,
            r#"INSERT INTO cached_files (object_id, object_type, message_id, chat_id)
            VALUES ($1, $2, $3, $4)
            RETURNING *"#,
            object_id,
            object_type,
            message_id,
            chat_id
        )
        .fetch_one(&db)
        .await
        .unwrap(),
    )
}

pub async fn download_from_cache(cached_data: CachedFile, db: Database) -> Option<DownloadResult> {
    let response_task = tokio::task::spawn(download_from_telegram_files(
        cached_data.message_id,
        cached_data.chat_id,
    ));
    let filename_task = tokio::task::spawn(get_filename(
        cached_data.object_id,
        cached_data.object_type.clone(),
    ));
    let book_task = tokio::task::spawn(get_book(cached_data.object_id));

    let response = match response_task.await.unwrap() {
        Ok(v) => {
            if v.status() != 200 {
                let cached_file_repo = CachedFileRepository::new(db.clone());

                let _ = cached_file_repo
                    .delete_by_object_id_object_type(
                        cached_data.object_id,
                        cached_data.object_type.clone(),
                    )
                    .await;

                return None;
            }

            v
        }
        Err(err) => {
            let cached_file_repo = CachedFileRepository::new(db.clone());

            let _ = cached_file_repo
                .delete_by_object_id_object_type(
                    cached_data.object_id,
                    cached_data.object_type.clone(),
                )
                .await;

            log::error!("{:?}", err);
            return None;
        }
    };

    let filename_data = match filename_task.await.unwrap() {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        }
    };

    let book = match book_task.await.unwrap() {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        }
    };

    let FilenameData {
        filename,
        filename_ascii,
    } = filename_data;
    let caption = book.get_caption();

    Some(DownloadResult {
        response,
        filename,
        filename_ascii,
        caption,
    })
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

    let first_page = match get_books(1, page_size, uploaded_gte.clone(), uploaded_lte.clone()).await
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
                r#"SELECT * FROM cached_files WHERE object_id = $1 AND object_type = $2"#,
                book.id,
                available_type.clone()
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

            cache_file(book.id, available_type, db.clone()).await;
        }
    }
}
