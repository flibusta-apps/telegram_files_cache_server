use once_cell::sync::Lazy;
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use std::error::Error;
use std::time::Duration;
use tokio::time::sleep;
use tracing::log;

use crate::config::CONFIG;

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

const MAX_RETRIES: u32 = 3;

fn is_transient_error(error: &reqwest::Error) -> bool {
    // Connect errors (DNS failures, connection refused)
    if error.is_connect() {
        return true;
    }

    // Timeouts
    if error.is_timeout() {
        return true;
    }

    // Request-phase errors include DNS resolution failures
    if error.is_request() {
        return true;
    }

    // Walk the source chain for hyper-level errors (incomplete message, etc.)
    let mut source = error.source();
    while let Some(err) = source {
        let desc = err.to_string();
        if desc.contains("IncompleteMessage")
            || desc.contains("Connection refused")
            || desc.contains("dns error")
            || desc.contains("failed to lookup address")
        {
            return true;
        }
        source = err.source();
    }

    false
}

async fn retry_transient<F, Fut, T>(f: F) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(err) => {
                let is_reqwest_error = err.downcast_ref::<reqwest::Error>().is_some();
                if is_reqwest_error && attempt < MAX_RETRIES - 1 {
                    let reqwest_err = err.downcast_ref::<reqwest::Error>().unwrap();
                    if is_transient_error(reqwest_err) {
                        attempt += 1;
                        let delay = Duration::from_secs(2u64.pow(attempt));
                        log::warn!(
                            "Transient error (attempt {}/{}): {}. Retrying in {:?}",
                            attempt,
                            MAX_RETRIES,
                            reqwest_err,
                            delay
                        );
                        sleep(delay).await;
                        continue;
                    }
                }
                return Err(err);
            }
        }
    }
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
) -> Result<Option<Response>, Box<dyn std::error::Error + Send + Sync>> {
    retry_transient(|| async {
        let url = format!(
            "{}/download/{source_id}/{remote_id}/{object_type}",
            CONFIG.downloader_url
        );

        let response = CLIENT
            .get(&url)
            .header("Authorization", &CONFIG.downloader_api_key)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
            .error_for_status()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

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
) -> Result<FilenameData, Box<dyn std::error::Error + Send + Sync>> {
    retry_transient(|| async {
        let url = format!(
            "{}/filename/{object_id}/{object_type}",
            CONFIG.downloader_url
        );

        let response = CLIENT
            .get(&url)
            .header("Authorization", &CONFIG.downloader_api_key)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
            .error_for_status()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let text = response
            .text()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
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
    })
    .await
}
