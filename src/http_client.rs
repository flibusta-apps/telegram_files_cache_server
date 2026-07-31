use std::time::Duration;

use once_cell::sync::Lazy;

/// Shared HTTP client for all outbound calls to the book_library, downloader, and
/// telegram_files backends. One configured client/connection-pool instead of three
/// separate ones.
pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("failed to build shared reqwest client")
});
