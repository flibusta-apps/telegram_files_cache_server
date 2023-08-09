use std::fmt;

use reqwest::{Response, multipart::{Form, Part}, header, StatusCode};
use serde::Deserialize;
use tracing::log;

use crate::config::CONFIG;


#[derive(Deserialize)]
pub struct UploadData {
    pub chat_id: i64,
    pub message_id: i64
}


#[derive(Deserialize)]
pub struct UploadResult {
    pub backend: String,
    pub data: UploadData
}


#[derive(Debug, Clone)]
struct DownloadError {
    status_code: StatusCode,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Status code is {0}", self.status_code)
    }
}

impl std::error::Error for DownloadError {}

pub async fn download_from_telegram_files(
    message_id: i64,
    chat_id: i64
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/api/v1/files/download_by_message/{chat_id}/{message_id}",
        CONFIG.files_url
    );

    let response = reqwest::Client::new()
        .get(url)
        .header("Authorization", CONFIG.files_api_key.clone())
        .send()
        .await?
        .error_for_status()?;

    if response.status() == StatusCode::NO_CONTENT {
        return Err(Box::new(DownloadError { status_code: StatusCode::NO_CONTENT }))
    };

    Ok(response)
}


pub async fn upload_to_telegram_files(
    data_response: Response,
    caption: String
) -> Result<UploadData, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/api/v1/files/upload/",
        CONFIG.files_url
    );

    let headers = data_response.headers();

    log::info!("{:?}", data_response.status());
    log::info!("{:?}", headers);

    let file_size = headers
        .get(header::CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let filename = headers
        .get("x-filename-b64-ascii")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let part = Part::stream(data_response)
        .file_name(filename);

    let form = Form::new()
        .text("caption", caption)
        .text("file_size", file_size)
        .part("file", part);

    let response = reqwest::Client::new()
        .post(url)
        .header("Authorization", CONFIG.files_api_key.clone())
        .multipart(form)
        .send()
        .await?
        .error_for_status()?;

    match response.json::<UploadResult>().await {
        Ok(v) => Ok(v.data),
        Err(err) => {
            Err(Box::new(err))
        },
    }
}
