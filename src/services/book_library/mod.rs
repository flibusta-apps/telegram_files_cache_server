pub mod types;

use serde::de::DeserializeOwned;

use crate::config::CONFIG;

use self::types::{BaseBook, Page};

async fn _make_request<T>(
    url: &str,
    params: Vec<(&str, String)>,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    T: DeserializeOwned,
{
    let client = reqwest::Client::new();

    let formated_url = format!("{}{}", CONFIG.library_url, url);

    let response = client
        .get(formated_url)
        .query(&params)
        .header("Authorization", CONFIG.library_api_key.clone())
        .send()
        .await;

    let response = match response {
        Ok(v) => v,
        Err(err) => return Err(Box::new(err)),
    };

    let response = match response.error_for_status() {
        Ok(v) => v,
        Err(err) => return Err(Box::new(err)),
    };

    match response.json::<T>().await {
        Ok(v) => Ok(v),
        Err(err) => Err(Box::new(err)),
    }
}

pub async fn get_sources() -> Result<types::Source, Box<dyn std::error::Error + Send + Sync>> {
    _make_request("/api/v1/sources", vec![]).await
}

pub async fn get_book(
    book_id: i32,
) -> Result<types::BookWithRemote, Box<dyn std::error::Error + Send + Sync>> {
    _make_request(format!("/api/v1/books/{book_id}").as_str(), vec![]).await
}

pub async fn get_books(
    page: u32,
    page_size: u32,
    uploaded_gte: String,
    uploaded_lte: String,
) -> Result<Page<BaseBook>, Box<dyn std::error::Error + Send + Sync>> {
    let params: Vec<(&str, String)> = vec![
        ("page", page.to_string()),
        ("size", page_size.to_string()),
        ("uploaded_gte", uploaded_gte),
        ("uploaded_lte", uploaded_lte)
    ];

    _make_request(format!("/api/v1/books/base/").as_str(), params).await
}
