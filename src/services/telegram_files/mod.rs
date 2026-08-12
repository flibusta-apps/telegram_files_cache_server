use std::time::Duration;

use base64::{engine::general_purpose, Engine};
use reqwest::{
    header,
    multipart::{Form, Part},
    Response,
};
use serde::Deserialize;

use crate::config::CONFIG;
use crate::http_client::CLIENT;
use crate::services::retry::{is_transient_error, retry_connect_only, retry_transient};

#[derive(Deserialize)]
pub struct UploadData {
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Deserialize)]
pub struct UploadResult {
    pub backend: String,
    pub data: UploadData,
}

pub async fn download_from_telegram_files(
    message_id: i64,
    chat_id: i64,
    max_retries: u32,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    retry_transient(max_retries, || async {
        let url = format!(
            "{}/api/v1/files/download_by_message/{chat_id}/{message_id}",
            CONFIG.files_url
        );

        let response = CLIENT
            .get(&url)
            .header("Authorization", CONFIG.files_api_key.clone())
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
            .error_for_status()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(response)
    })
    .await
}

/// Payload for `upload_bytes_to_telegram_files`: fully-buffered file bytes plus the
/// metadata needed for the multipart upload to the internal `files_server`.
pub struct UploadPayload {
    pub bytes: bytes::Bytes,
    pub filename: String,
    pub caption: String,
}

/// Uploads already-buffered bytes to the internal `files_server`. Used both by the cold
/// path (`upload_to_telegram_files`, after buffering a downloader `Response`) and by the
/// cross-normalized reuse path (`services::reuse`), which never has a downloader
/// `Response` to begin with.
pub async fn upload_bytes_to_telegram_files(
    payload: UploadPayload,
    max_retries: u32,
) -> Result<UploadData, Box<dyn std::error::Error + Send + Sync>> {
    let file_size = payload.bytes.len().to_string();
    let filename = payload.filename;
    let caption = payload.caption;
    let body_bytes = payload.bytes;

    let file_size_clone = file_size.clone();
    let filename_clone = filename.clone();
    let caption_clone = caption.clone();
    let body_bytes_clone = body_bytes.clone();

    let result = retry_connect_only(max_retries, move || {
        let body_bytes = body_bytes_clone.clone();
        let filename = filename_clone.clone();
        let file_size = file_size_clone.clone();
        let caption = caption_clone.clone();

        async move {
            let url = format!("{}/api/v1/files/upload/", CONFIG.files_url);

            let part = Part::stream(body_bytes.clone()).file_name(filename.clone());

            let form = Form::new()
                .text("caption", caption)
                .text("file_size", file_size)
                .text("filename", filename)
                .part("file", part);

            let response = CLIENT
                .post(&url)
                .header("Authorization", CONFIG.files_api_key.clone())
                .multipart(form)
                .send()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .error_for_status()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let text = response
                .text()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            match serde_json::from_str::<UploadResult>(&text) {
                Ok(v) => Ok(v.data),
                Err(err) => {
                    tracing::error!(
                        "Failed to decode UploadResult from files server: {}. Response body: {:?}",
                        err,
                        text
                    );
                    Err(Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
                }
            }
        }
    })
    .await;

    if let Err(err) = &result {
        if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
            if is_transient_error(reqwest_err) && !reqwest_err.is_connect() {
                tracing::warn!(
                    "Upload to files server failed with a non-connect transient error and was not retried \
                     (to avoid duplicating a Telegram message if the first attempt actually succeeded \
                     server-side): {}",
                    reqwest_err
                );
            }
        }
    }

    result
}

pub async fn upload_to_telegram_files(
    data_response: Response,
    caption: String,
    max_retries: u32,
) -> Result<UploadData, Box<dyn std::error::Error + Send + Sync>> {
    // Extract data from Response before retry loop (Response can only be consumed once)
    let headers = data_response.headers().clone();

    // Validated for parity with prior behavior even though the byte count used for the
    // upload is now derived from the buffered body itself (`payload.bytes.len()` in
    // `upload_bytes_to_telegram_files`), not this header value.
    headers.get(header::CONTENT_LENGTH).ok_or_else(|| {
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing Content-Length",
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let base64_encoder = general_purpose::STANDARD;

    let filename = std::str::from_utf8(
        &base64_encoder
            .decode(headers.get("x-filename-b64-ascii").ok_or_else(|| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing x-filename-b64-ascii header",
                )) as Box<dyn std::error::Error + Send + Sync>
            })?)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?,
    )
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
    .to_string();

    let bytes = data_response
        .bytes()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    upload_bytes_to_telegram_files(
        UploadPayload {
            bytes,
            filename,
            caption,
        },
        max_retries,
    )
    .await
}
