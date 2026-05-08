use std::error::Error;
use std::time::Duration;
use tokio::time::sleep;
use tracing::log;

pub const MAX_RETRIES: u32 = 3;

pub fn is_transient_error(error: &reqwest::Error) -> bool {
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

    // HTTP 5xx server errors are transient
    if error.is_status() {
        if let Some(status) = error.status() {
            if status.is_server_error() {
                return true;
            }
        }
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

pub async fn retry_transient<F, Fut, T>(f: F) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
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
