# Spec 08: Observability — Error Context, Cache Metrics, Log Configuration

- **Priority:** medium
- **Effort:** S
- **Category:** observability

## Problem(s)

### 08.1 Errors are logged without any request context
Every failure site logs only the raw error: `log::error!("{:?}", err)` at `src/services/mod.rs:143,161,172,236,244,252,327,346`. None include `object_id`, `object_type`, `is_normalized`, or the upstream URL, so a Sentry event ("error decoding response body") cannot be traced back to a specific book or route. Errors that turn into `None` → 204 (Spec 01.1) leave *no* trace at all of which request was affected.

**Fix:** Use `tracing::error!` with structured fields (`object_id`, `object_type`, `upstream = "downloader"`, etc.), or instrument service functions with `#[tracing::instrument]` so fields propagate to Sentry.

### 08.2 No cache hit/miss/eviction metrics
The whole purpose of the service is caching, yet there is no metric distinguishing a hit (`src/services/mod.rs:74-75`) from a miss+fill (`line 76`) or an eviction (`lines 210-238`). `axum-prometheus` (`src/views.rs:197`) only provides generic HTTP counters. Capacity/effectiveness of the cache is invisible; a spike of evictions (e.g. telegram_files degradation deleting rows per Spec 03.5) would go unnoticed.

**Fix:** Add counters `cache_hits_total`, `cache_misses_total`, `cache_evictions_total{reason}`, `upload_failures_total` via `metrics::counter!` (axum-prometheus already brings the `metrics` facade).

### 08.3 Log level is hardcoded; `env-filter` feature is enabled but unused
`src/main.rs:35-39` installs a fixed `filter::LevelFilter::INFO`. Cargo.toml line 35 enables `tracing-subscriber`'s `env-filter` feature, but `EnvFilter`/`RUST_LOG` is never wired, so operators cannot raise verbosity (e.g. debug a single deployment) without a rebuild.

**Fix:** `EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())`.

### 08.4 Background task outcomes are invisible
`start_update_cache` (`src/services/mod.rs:322-358`) reports nothing on completion: no summary log (books scanned, files cached, failures), no duration, no metric. Combined with the fire-and-forget endpoint (`src/views.rs:156-160`), an operator cannot tell whether cache warming works at all.

**Fix:** Log a structured summary at completion and export `update_cache_runs_total`/`update_cache_cached_files_total` counters + duration histogram.

## Acceptance criteria
- Every error log/Sentry event from the request path carries `object_id` and `object_type` fields.
- `/metrics` exposes cache hit/miss/eviction counters that change under exercised traffic.
- `RUST_LOG=debug` raises log verbosity without recompiling.
- Each `update_cache` run emits a completion log with counts and duration.
