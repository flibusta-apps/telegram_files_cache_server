use std::error::Error;

use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use reqwest::Response as HttpResponse;

pub type BoxError = Box<dyn Error + Send + Sync>;

/// Default `Retry-After` (seconds) used when an upstream 429 carries no usable hint.
pub const DEFAULT_RETRY_AFTER_SECS: u64 = 10;

/// An upstream (downloader / files server / Telegram) answered 429. Kept as a typed
/// error so it can be propagated to our callers as 429 + `Retry-After`.
#[derive(Debug)]
pub struct RateLimitedError {
    pub retry_after_secs: u64,
}

impl std::fmt::Display for RateLimitedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "upstream rate limited, retry after {}s",
            self.retry_after_secs
        )
    }
}

impl Error for RateLimitedError {}

/// Error shared between several waiters of a single-flight operation. Preserves the
/// original error as `source()` so typed lookups (rate limit, db) still work.
#[derive(Debug, Clone)]
pub struct SharedError(pub std::sync::Arc<BoxError>);

impl std::fmt::Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for SharedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&**self.0)
    }
}

fn chain<'a>(err: &'a (dyn Error + 'static)) -> impl Iterator<Item = &'a (dyn Error + 'static)> {
    std::iter::successors(Some(err), |e: &&'a (dyn Error + 'static)| (*e).source())
}

/// If anywhere in the error chain there is an upstream 429, returns the Retry-After secs.
pub fn rate_limit_secs(err: &(dyn Error + 'static)) -> Option<u64> {
    for e in chain(err) {
        if let Some(rl) = e.downcast_ref::<RateLimitedError>() {
            return Some(rl.retry_after_secs);
        }
        if let Some(teloxide::RequestError::RetryAfter(secs)) =
            e.downcast_ref::<teloxide::RequestError>()
        {
            return Some(u64::from(secs.seconds()).max(1));
        }
        if let Some(re) = e.downcast_ref::<reqwest::Error>() {
            if re.status() == Some(StatusCode::TOO_MANY_REQUESTS) {
                return Some(DEFAULT_RETRY_AFTER_SECS);
            }
        }
    }
    None
}

pub fn is_db_error(err: &(dyn Error + 'static)) -> bool {
    chain(err).any(|e| e.downcast_ref::<sqlx::Error>().is_some())
}

#[derive(serde::Deserialize)]
struct RateLimitBody {
    retry_after_secs: Option<u64>,
}

/// Like `error_for_status`, but a 429 becomes a typed [`RateLimitedError`] carrying the
/// upstream `Retry-After` (header first, JSON body `retry_after_secs` second).
pub async fn check_status(response: HttpResponse) -> Result<HttpResponse, BoxError> {
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        let from_header = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let secs = match from_header {
            Some(v) => v,
            None => response
                .text()
                .await
                .ok()
                .and_then(|t| serde_json::from_str::<RateLimitBody>(&t).ok())
                .and_then(|b| b.retry_after_secs)
                .unwrap_or(DEFAULT_RETRY_AFTER_SECS),
        };
        return Err(Box::new(RateLimitedError {
            retry_after_secs: secs.max(1),
        }));
    }
    response
        .error_for_status()
        .map_err(|e| Box::new(e) as BoxError)
}

/// Maps a service error to an HTTP response: upstream 429 → 429 + Retry-After + JSON,
/// local DB failure → 500, everything else → 502.
pub fn error_response(err: &(dyn Error + 'static)) -> Response {
    if let Some(secs) = rate_limit_secs(err) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, secs.to_string())],
            Json(serde_json::json!({
                "detail": "Upstream rate limit exceeded",
                "retry_after_secs": secs,
            })),
        )
            .into_response();
    }
    if is_db_error(err) {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    StatusCode::BAD_GATEWAY.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_maps_to_429_with_header() {
        let err: BoxError = Box::new(RateLimitedError {
            retry_after_secs: 7,
        });
        let resp = error_response(&*err);
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers().get(header::RETRY_AFTER).unwrap(), "7");
    }

    #[test]
    fn shared_error_keeps_rate_limit() {
        let inner: BoxError = Box::new(RateLimitedError {
            retry_after_secs: 3,
        });
        let shared = SharedError(std::sync::Arc::new(inner));
        assert_eq!(rate_limit_secs(&shared), Some(3));
    }

    #[test]
    fn db_error_maps_to_500() {
        let err: BoxError = Box::new(sqlx::Error::PoolTimedOut);
        assert_eq!(
            error_response(&*err).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn other_error_maps_to_502() {
        let err: BoxError = Box::new(std::io::Error::other("x"));
        assert_eq!(error_response(&*err).status(), StatusCode::BAD_GATEWAY);
    }
}
