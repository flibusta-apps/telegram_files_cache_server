use base64::{engine::general_purpose, Engine};
use reqwest::{
    header,
    multipart::{Form, Part},
    Response,
};
use serde::Deserialize;

use crate::config::CONFIG;

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

    Ok(response)
}

pub async fn upload_to_telegram_files(
    data_response: Response,
    caption: String,
) -> Result<UploadData, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}/api/v1/files/upload/", CONFIG.files_url);

    let headers = data_response.headers();

    let file_size = headers
        .get(header::CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let base64_encoder = general_purpose::STANDARD;

    let filename = std::str::from_utf8(
        &base64_encoder
            .decode(headers.get("x-filename-b64-ascii").unwrap())
            .unwrap(),
    )
    .unwrap()
    .to_string();

    let part = Part::stream(data_response).file_name(filename.clone());

    let form = Form::new()
        .text("caption", caption)
        .text("file_size", file_size)
        .text("filename", filename)
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
        Err(err) => Err(Box::new(err)),
    }
}
