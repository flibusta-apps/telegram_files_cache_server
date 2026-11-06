//! Shared by HTTP misses and warmup through `cache_file`, before any file download.
use once_cell::sync::Lazy;
use tokio::sync::Semaphore;

pub static FILLS: Lazy<Semaphore> = Lazy::new(|| {
    Semaphore::new(fill_concurrency(
        std::env::var("MAX_CONCURRENT_FILLS").ok().as_deref(),
    ))
});

fn fill_concurrency(value: Option<&str>) -> usize {
    value
        .and_then(|v| v.parse().ok())
        .filter(|v| *v > 0 && *v <= Semaphore::MAX_PERMITS)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_is_positive_and_bounded() {
        for value in [None, Some("0"), Some("-1"), Some("invalid")] {
            assert_eq!(fill_concurrency(value), 4);
        }
        assert_eq!(fill_concurrency(Some("2")), 2);
        assert_eq!(fill_concurrency(Some(&usize::MAX.to_string())), 4);
    }

    #[tokio::test]
    async fn fills_wait_and_cancellation_releases_capacity() {
        let semaphore = std::sync::Arc::new(Semaphore::new(2));
        let first = semaphore.clone().acquire_owned().await.unwrap();
        let second = semaphore.clone().acquire_owned().await.unwrap();
        assert!(semaphore.try_acquire().is_err());
        let waiting = semaphore.acquire();
        tokio::pin!(waiting);
        assert!(futures::poll!(&mut waiting).is_pending());
        drop(first);
        let acquired = waiting.await.unwrap();
        assert!(semaphore.try_acquire().is_err());
        drop(acquired);
        let task = tokio::spawn(async move {
            let _permit = second;
            std::future::pending::<()>().await;
        });
        task.abort();
        let _ = task.await;
        assert_eq!(semaphore.available_permits(), 2);
    }
}
