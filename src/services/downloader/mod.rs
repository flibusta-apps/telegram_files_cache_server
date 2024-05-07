use reqwest::{Response, StatusCode};
use serde::Deserialize;

use crate::config::CONFIG;

#[derive(Deserialize)]
pub struct FilenameData {
    pub filename: String,
    pub filename_ascii: String,
}

pub async fn download_from_downloader(
    source_id: u32,
    remote_id: u32,
    object_type: String,
) -> Result<Option<Response>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/download/{source_id}/{remote_id}/{object_type}",
        CONFIG.downloader_url
    );

    let response = reqwest::Client::new()
        .get(url)
        .header("Authorization", &CONFIG.downloader_api_key)
        .send()
        .await?
        .error_for_status()?;

    if response.status() == StatusCode::NO_CONTENT {
        return Ok(None);
    };

    Ok(Some(response))
}

pub async fn get_filename(
    object_id: i32,
    object_type: String,
) -> Result<FilenameData, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "{}/filename/{object_id}/{object_type}",
        CONFIG.downloader_url
    );

    let response = reqwest::Client::new()
        .get(url)
        .header("Authorization", &CONFIG.downloader_api_key)
        .send()
        .await?
        .error_for_status()?;

    match response.json::<FilenameData>().await {
        Ok(v) => Ok(v),
        Err(err) => Err(Box::new(err)),
    }
}
