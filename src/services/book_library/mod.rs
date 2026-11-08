pub mod types;

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::config::CONFIG;
use crate::http_client::CLIENT;
use crate::services::retry::retry_transient;

use self::types::{BaseBook, Page};

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
                .timeout(Duration::from_secs(15))
                .header("Authorization", CONFIG.library_api_key.clone())
                .send()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(Box::new(crate::error::NotFoundError) as crate::error::BoxError);
            }
            let response = crate::error::check_status(response).await?;

            let text = response
                .text()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            match serde_json::from_str::<T>(&text) {
                Ok(v) => Ok(v),
                Err(err) => {
                    tracing::error!(
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

    _make_request("/api/v1/books/base", params, max_retries).await
}

#[cfg(test)]
mod dependency_tests {
    #[test]
    fn reqwest_query_preserves_pagination_and_timestamp_encoding() {
        let params = vec![
            ("page", "2".to_owned()),
            ("size", "100".to_owned()),
            ("uploaded_gte", "2026-11-08T12:30:00+03:00".to_owned()),
            ("uploaded_lte", "2026-11-09 12:30:00".to_owned()),
        ];
        let request = reqwest::Client::new()
            .get("http://localhost/api/v1/books/base")
            .query(&params)
            .build()
            .unwrap();
        let pairs: Vec<_> = request.url().query_pairs().collect();
        assert_eq!(pairs.len(), params.len());
        for ((key, value), (expected_key, expected_value)) in pairs.iter().zip(&params) {
            assert_eq!(key, expected_key);
            assert_eq!(value, expected_value);
        }
        assert!(request.url().as_str().contains("%2B03%3A00"));
    }
}
