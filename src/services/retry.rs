use std::error::Error;
use std::time::Duration;

use rand::Rng;
use tokio::time::sleep;
use tracing::log;

/// Retry budget for calls made inline in a user-facing HTTP handler:
/// 1 retry (2 total attempts) so a struggling upstream doesn't stall the request for long.
pub const INTERACTIVE_MAX_RETRIES: u32 = 2;
/// Retry budget for background jobs (cache warmup): preserves the previous default
/// of up to 2 retries (3 total attempts).
pub const BACKGROUND_MAX_RETRIES: u32 = 3;

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

/// Only true for errors that occur before any request bytes could have reached the
/// server (DNS/connect failures). Safe to retry even for non-idempotent requests
/// (e.g. file uploads), since the server could not have received/processed anything.
pub fn is_connect_phase_error(error: &reqwest::Error) -> bool {
    error.is_connect()
}

async fn retry_with<F, Fut, T, P>(
    max_retries: u32,
    is_retryable: P,
    f: F,
) -> Result<T, Box<dyn Error + Send + Sync>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
    P: Fn(&reqwest::Error) -> bool,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(err) => {
                let is_reqwest_error = err.downcast_ref::<reqwest::Error>().is_some();
                if is_reqwest_error && attempt < max_retries - 1 {
                    let reqwest_err = err.downcast_ref::<reqwest::Error>().unwrap();
                    if is_retryable(reqwest_err) {
                        attempt += 1;
                        let base = Duration::from_secs(2u64.pow(attempt));
                        let jitter_ms = rand::thread_rng().gen_range(0..500u64);
                        let delay = base + Duration::from_millis(jitter_ms);
                        log::warn!(
                            "Transient error (attempt {}/{}): {}. Retrying in {:?}",
                            attempt,
                            max_retries,
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

/// Retries on connect/timeout/request-phase errors and 5xx responses.
/// Use for idempotent (read-only) upstream calls.
pub async fn retry_transient<F, Fut, T>(
    max_retries: u32,
    f: F,
) -> Result<T, Box<dyn Error + Send + Sync>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
{
    retry_with(max_retries, is_transient_error, f).await
}

/// Retries only on connect-phase errors. Use for non-idempotent calls (e.g. file
/// upload) where retrying after the request may have already reached the server
/// (timeout while waiting for response, dropped connection mid-response, 5xx) could
/// silently duplicate a side effect.
pub async fn retry_connect_only<F, Fut, T>(
    max_retries: u32,
    f: F,
) -> Result<T, Box<dyn Error + Send + Sync>>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, Box<dyn Error + Send + Sync>>>,
{
    retry_with(max_retries, is_connect_phase_error, f).await
}
