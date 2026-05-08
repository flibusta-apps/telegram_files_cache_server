use base64::{engine::general_purpose, Engine};
use once_cell::sync::Lazy;
use reqwest::{
    header,
    multipart::{Form, Part},
    Response,
};
use serde::Deserialize;
use tracing::log;

use crate::config::CONFIG;
use crate::services::retry::retry_transient;

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

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
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    retry_transient(|| async {
        let url = format!(
            "{}/api/v1/files/download_by_message/{chat_id}/{message_id}",
            CONFIG.files_url
        );

        let response = CLIENT
            .get(&url)
            .header("Authorization", CONFIG.files_api_key.clone())
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
            .error_for_status()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(response)
    })
    .await
}

pub async fn upload_to_telegram_files(
    data_response: Response,
    caption: String,
) -> Result<UploadData, Box<dyn std::error::Error + Send + Sync>> {
    // Extract data from Response before retry loop (Response can only be consumed once)
    let headers = data_response.headers().clone();

    let file_size = headers
        .get(header::CONTENT_LENGTH)
        .ok_or_else(|| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing Content-Length",
            )) as Box<dyn std::error::Error + Send + Sync>
        })?
        .to_str()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        .to_string();

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

    let body_bytes = data_response
        .bytes()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    let file_size_clone = file_size.clone();
    let filename_clone = filename.clone();
    let caption_clone = caption.clone();
    let body_bytes_clone = body_bytes.clone();

    retry_transient(move || {
        let body_bytes = body_bytes_clone.clone();
        let filename = filename_clone.clone();
        let file_size = file_size_clone.clone();
        let caption = caption_clone.clone();

        async move {
            let url = format!("{}/api/v1/files/upload/", CONFIG.files_url);

            let part = Part::bytes(body_bytes.to_vec()).file_name(filename.clone());

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
                    log::error!(
                        "Failed to decode UploadResult from files server: {}. Response body: {:?}",
                        err,
                        text
                    );
                    Err(Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
                }
            }
        }
    })
    .await
}
