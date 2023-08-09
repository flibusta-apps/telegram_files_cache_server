pub mod book_library;
pub mod download_utils;
pub mod telegram_files;
pub mod downloader;

use tracing::log;

use crate::{prisma::cached_file, views::Database};

use self::{download_utils::DownloadResult, telegram_files::{download_from_telegram_files, UploadData, upload_to_telegram_files}, downloader::{get_filename, FilenameData, download_from_downloader}, book_library::get_book};


pub async fn get_cached_file_or_cache(
    object_id: i32,
    object_type: String,
    db: Database
) -> Option<cached_file::Data> {
    let cached_file = db.cached_file()
        .find_unique(cached_file::object_id_object_type(object_id, object_type.clone()))
        .exec()
        .await
        .unwrap();

    match cached_file {
        Some(cached_file) => Some(cached_file),
        None => cache_file(object_id, object_type, db).await,
    }
}


pub async fn cache_file(
    object_id: i32,
    object_type: String,
    db: Database
) -> Option<cached_file::Data> {
    let book = match get_book(object_id).await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        },
    };

    let downloader_result = match download_from_downloader(
        book.remote_id,
        object_id,
        object_type.clone()
    ).await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        },
    };

    let UploadData { chat_id, message_id } = match upload_to_telegram_files(
        downloader_result,
        book.get_caption()
    ).await {
        Ok(v) => v,
        Err(err) => {
            log::error!("{:?}", err);
            return None;
        },
    };

    Some(
        db
        .cached_file()
        .create(
            object_id,
            object_type,
            message_id,
            chat_id,
            vec![]
        )
        .exec()
        .await
        .unwrap()
    )
}


pub async fn download_from_cache(
    cached_data: cached_file::Data,
    db: Database
) -> Option<DownloadResult> {
    let response_task = tokio::task::spawn(download_from_telegram_files(cached_data.message_id, cached_data.chat_id));
    let filename_task = tokio::task::spawn(get_filename(cached_data.object_id, cached_data.object_type.clone()));
    let book_task = tokio::task::spawn(get_book(cached_data.object_id));

    let response = match response_task.await.unwrap() {
        Ok(v) => v,
        Err(err) => {
            db.cached_file()
                .delete(cached_file::object_id_object_type(cached_data.object_id, cached_data.object_type.clone()))
                .exec()
                .await
                .unwrap();

            tokio::spawn(cache_file(cached_data.object_id, cached_data.object_type, db));

            log::error!("{:?}", err);
            return None;
        },
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

    let FilenameData {filename, filename_ascii} = filename_data;
    let caption = book.get_caption();

    Some(DownloadResult {
        response,
        filename,
        filename_ascii,
        caption
    })
}
