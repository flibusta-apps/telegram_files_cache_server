pub mod types;

use std::time::Duration;

use once_cell::sync::Lazy;
use serde::de::DeserializeOwned;
use tracing::log;

use crate::config::CONFIG;
use crate::services::retry::retry_transient;

use self::types::{BaseBook, Page};

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to build book_library reqwest client")
});

async fn _make_request<T>(
    url: &str,
    params: Vec<(&str, String)>,
    max_retries: u32,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    T: DeserializeOwned,
{
    let url_owned = url.to_string();
    retry_transient(max_retries, || {
        let url = url_owned.clone();
        let params = params.clone();
        async move {
            let formated_url = format!("{}{}", CONFIG.library_url, url);

            let response = CLIENT
                .get(&formated_url)
                .query(&params)
                .header("Authorization", CONFIG.library_api_key.clone())
                .send()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let response = response
                .error_for_status()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let text = response
                .text()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            match serde_json::from_str::<T>(&text) {
                Ok(v) => Ok(v),
                Err(err) => {
                    log::error!(
                        "Failed to decode {} from library: {}. Response body: {:?}",
                        std::any::type_name::<T>(),
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

pub async fn get_sources() -> Result<types::Source, Box<dyn std::error::Error + Send + Sync>> {
    _make_request(
        "/api/v1/sources",
        vec![],
        crate::services::retry::BACKGROUND_MAX_RETRIES,
    )
    .await
}

pub async fn get_book(
    book_id: i32,
    max_retries: u32,
) -> Result<types::BookWithRemote, Box<dyn std::error::Error + Send + Sync>> {
    _make_request(
        format!("/api/v1/books/{book_id}").as_str(),
        vec![],
        max_retries,
    )
    .await
}

pub async fn get_books(
    page: u32,
    page_size: u32,
    uploaded_gte: String,
    uploaded_lte: String,
    max_retries: u32,
) -> Result<Page<BaseBook>, Box<dyn std::error::Error + Send + Sync>> {
    let params: Vec<(&str, String)> = vec![
        ("page", page.to_string()),
        ("size", page_size.to_string()),
        ("uploaded_gte", uploaded_gte),
        ("uploaded_lte", uploaded_lte),
    ];

    _make_request("/api/v1/books/base/", params, max_retries).await
}
