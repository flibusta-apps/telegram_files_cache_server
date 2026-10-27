use std::time::Duration;

use reqwest::{Response, StatusCode};
use serde::Deserialize;

use crate::config::CONFIG;
use crate::error::{check_status, BoxError};
use crate::http_client::CLIENT;
use crate::services::retry::retry_transient;

/// Builds `<base>/<segments...>` encoding every segment as a single path segment.
fn build_url(segments: &[&str], query: &[(&str, &str)]) -> Result<reqwest::Url, BoxError> {
    let mut url = reqwest::Url::parse(&CONFIG.downloader_url)?;
    url.path_segments_mut()
        .map_err(|_| std::io::Error::other("DOWNLOADER_URL cannot be a base"))?
        .pop_if_empty()
        .extend(segments);
    url.query_pairs_mut().extend_pairs(query);
    Ok(url)
}

#[derive(Deserialize)]
pub struct FilenameData {
    pub filename: String,
    pub filename_ascii: String,
}

pub async fn download_from_downloader(
    source_id: u32,
    remote_id: u32,
    object_type: String,
    is_normalized: bool,
    max_retries: u32,
) -> Result<Option<Response>, Box<dyn std::error::Error + Send + Sync>> {
    retry_transient(max_retries, || async {
        let url = build_url(
            &[
                "download",
                &source_id.to_string(),
                &remote_id.to_string(),
                &object_type,
            ],
            &[("normalized", if is_normalized { "true" } else { "false" })],
        )?;

        let response = CLIENT
            .get(url)
            .header("Authorization", &CONFIG.downloader_api_key)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let response = check_status(response).await?;

        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        };

        Ok(Some(response))
    })
    .await
}

pub async fn get_filename(
    object_id: i32,
    object_type: String,
    is_normalized: bool,
    max_retries: u32,
) -> Result<FilenameData, Box<dyn std::error::Error + Send + Sync>> {
    retry_transient(max_retries, || async {
        let url = build_url(
            &["filename", &object_id.to_string(), &object_type],
            &[("normalized", if is_normalized { "true" } else { "false" })],
        )?;

        let response = CLIENT
            .get(url)
            .header("Authorization", &CONFIG.downloader_api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let response = check_status(response).await?;

        let text = response
            .text()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        match serde_json::from_str::<FilenameData>(&text) {
            Ok(v) => Ok(v),
            Err(err) => {
                tracing::error!(
                    "Failed to decode FilenameData from downloader: {}. Response body: {:?}",
                    err,
                    text
                );
                Err(Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
            }
        }
    })
    .await
}
