use std::sync::Arc;

use futures::stream::{self, StreamExt};

/// Fixed backoff applied to the whole warmup stream after a 429 from the files server.
/// We don't have a `Retry-After` header available at this point (responses go through
/// `error_for_status()` upstream, which discards headers on the error path), so a
/// conservative fixed pause is used instead of trying to parse one.
pub const WARMUP_429_BACKOFF: std::time::Duration = std::time::Duration::from_secs(30);

/// Shared backoff gate used to pause every in-flight warmup worker when any one of them
/// hits a 429. `wait_if_paused` is checked by every worker before doing real work;
/// `trigger_backoff` extends the shared pause (never shortens an existing, longer one).
pub struct WarmupThrottle {
    resume_at: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl WarmupThrottle {
    pub fn new() -> Self {
        Self {
            resume_at: tokio::sync::Mutex::new(None),
        }
    }

    /// Blocks until any in-flight backoff has elapsed. Re-checks after waking in case
    /// another task extended the backoff while this one was asleep.
    pub async fn wait_if_paused(&self) {
        loop {
            let deadline = *self.resume_at.lock().await;
            match deadline {
                Some(instant) if instant > tokio::time::Instant::now() => {
                    tokio::time::sleep_until(instant).await;
                }
                _ => break,
            }
        }
    }

    /// Extends the shared pause to at least `now + duration` (never shortens an existing,
    /// longer pause).
    pub async fn trigger_backoff(&self, duration: std::time::Duration) {
        let new_resume = tokio::time::Instant::now() + duration;
        let mut guard = self.resume_at.lock().await;
        if guard.is_none_or(|cur| new_resume > cur) {
            *guard = Some(new_resume);
        }
    }
}

impl Default for WarmupThrottle {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `worker` over `items` with at most `concurrency` invocations in flight at once.
/// Each invocation first waits on `throttle.wait_if_paused()` so a backoff triggered by
/// any sibling pauses the whole stream, not just the task that hit the 429. Returns
/// `(successes, failures)` counted from the worker's `bool` result (`true` = success).
pub async fn run_with_concurrency<T, F, Fut>(
    items: Vec<T>,
    concurrency: usize,
    throttle: Arc<WarmupThrottle>,
    worker: F,
) -> (usize, usize)
where
    T: Send + 'static,
    F: Fn(T, Arc<WarmupThrottle>) -> Fut + Send + Sync + 'static + Clone,
    Fut: std::future::Future<Output = bool> + Send,
{
    stream::iter(items)
        .map(|item| {
            let throttle = throttle.clone();
            let worker = worker.clone();
            async move {
                throttle.wait_if_paused().await;
                worker(item, throttle).await
            }
        })
        .buffer_unordered(concurrency)
        .fold((0usize, 0usize), |(ok, fail), success| async move {
            if success {
                (ok + 1, fail)
            } else {
                (ok, fail + 1)
            }
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn warmup_throttle_wait_if_paused_returns_immediately_with_no_backoff() {
        let throttle = WarmupThrottle::new();
        let start = tokio::time::Instant::now();
        throttle.wait_if_paused().await;
        assert_eq!(tokio::time::Instant::now(), start);
    }

    #[tokio::test(start_paused = true)]
    async fn warmup_throttle_blocks_until_backoff_elapses() {
        let throttle = Arc::new(WarmupThrottle::new());
        throttle.trigger_backoff(Duration::from_secs(5)).await;

        let start = tokio::time::Instant::now();
        let throttle2 = throttle.clone();
        let waiter = tokio::spawn(async move {
            throttle2.wait_if_paused().await;
            tokio::time::Instant::now()
        });

        tokio::time::sleep(Duration::from_secs(1)).await;
        // Not resolved yet.
        assert!(!waiter.is_finished());

        let resumed_at = waiter.await.unwrap();
        assert!(resumed_at >= start + Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn warmup_throttle_second_shorter_backoff_does_not_shorten_existing_pause() {
        let throttle = Arc::new(WarmupThrottle::new());
        throttle.trigger_backoff(Duration::from_secs(5)).await;
        throttle.trigger_backoff(Duration::from_secs(1)).await;

        let start = tokio::time::Instant::now();
        throttle.wait_if_paused().await;
        let resumed_at = tokio::time::Instant::now();

        assert!(resumed_at >= start + Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn run_with_concurrency_speeds_up_with_higher_concurrency() {
        async fn sleep_worker(_item: usize, _throttle: Arc<WarmupThrottle>) -> bool {
            tokio::time::sleep(Duration::from_millis(200)).await;
            true
        }

        let items: Vec<usize> = (0..60).collect();

        let throttle1 = Arc::new(WarmupThrottle::new());
        let start1 = tokio::time::Instant::now();
        let (ok1, fail1) = run_with_concurrency(items.clone(), 1, throttle1, sleep_worker).await;
        let elapsed_c1 = tokio::time::Instant::now() - start1;
        assert_eq!(ok1, 60);
        assert_eq!(fail1, 0);

        let throttle4 = Arc::new(WarmupThrottle::new());
        let start4 = tokio::time::Instant::now();
        let (ok4, fail4) = run_with_concurrency(items, 4, throttle4, sleep_worker).await;
        let elapsed_c4 = tokio::time::Instant::now() - start4;
        assert_eq!(ok4, 60);
        assert_eq!(fail4, 0);

        assert!(
            elapsed_c4 * 3 <= elapsed_c1,
            "expected concurrency=4 ({:?}) to be at least 3x faster than concurrency=1 ({:?})",
            elapsed_c4,
            elapsed_c1
        );
    }

    #[tokio::test(start_paused = true)]
    async fn run_with_concurrency_429_backoff_pauses_all_in_flight() {
        // Items 0..4 are "trigger" items that fail and trigger a backoff. Items 4..20
        // are normal items that should be delayed until the backoff elapses if they
        // start waiting after the backoff was triggered.
        let backoff = Duration::from_millis(500);
        let start_times: Arc<std::sync::Mutex<Vec<(usize, tokio::time::Instant)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let trigger_count = Arc::new(AtomicUsize::new(0));

        let items: Vec<usize> = (0..20).collect();
        let throttle = Arc::new(WarmupThrottle::new());
        let start = tokio::time::Instant::now();

        let start_times_worker = start_times.clone();
        let trigger_count_worker = trigger_count.clone();
        let worker = move |item: usize, throttle: Arc<WarmupThrottle>| {
            let start_times = start_times_worker.clone();
            let trigger_count = trigger_count_worker.clone();
            async move {
                start_times
                    .lock()
                    .unwrap()
                    .push((item, tokio::time::Instant::now()));

                if item < 4 {
                    trigger_count.fetch_add(1, Ordering::SeqCst);
                    throttle.trigger_backoff(backoff).await;
                    false
                } else {
                    // Simulate some real work so other tasks have a chance to enter
                    // wait_if_paused concurrently.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    true
                }
            }
        };

        let (ok, fail) = run_with_concurrency(items, 4, throttle, worker).await;
        assert_eq!(fail, 4);
        assert_eq!(ok, 16);

        let recorded = start_times.lock().unwrap().clone();
        // Every item's recorded start time (i.e. the moment it began doing real work,
        // after wait_if_paused) that comes chronologically after the backoff-triggering
        // items were dispatched must be at or after `start + backoff` — i.e. no item
        // slips through mid-pause.
        let backoff_deadline = start + backoff;
        for (item, at) in recorded {
            if item >= 4 {
                // Items scheduled in later concurrency batches (after the first 4 items
                // triggered backoff) must have waited for the pause to elapse.
                assert!(
                    at >= backoff_deadline || item < 4,
                    "item {} started at {:?} before backoff deadline {:?}",
                    item,
                    at,
                    backoff_deadline
                );
            }
        }
    }
}
