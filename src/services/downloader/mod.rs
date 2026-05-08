use once_cell::sync::Lazy;
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use tracing::log;

use crate::config::CONFIG;

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

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

    let response = CLIENT
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

    let response = CLIENT
        .get(url)
        .header("Authorization", &CONFIG.downloader_api_key)
        .send()
        .await?
        .error_for_status()?;

    let text = response.text().await?;
    match serde_json::from_str::<FilenameData>(&text) {
        Ok(v) => Ok(v),
        Err(err) => {
            log::error!(
                "Failed to decode FilenameData from downloader: {}. Response body: {:?}",
                err,
                text
            );
            Err(Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
        }
    }
}
