# Audit Specs Index — telegram_files_cache_server

Full audit performed 2026-07-07 against the current working tree (~1360 LOC Rust).
Consumers considered: `book_bot` (panics if `x-filename-b64`/`x-caption-b64` missing on 200; treats 204 as "no file") and `batch_downloader`.

| Spec | Title | Priority | Effort | Category |
|------|-------|----------|--------|----------|
| [01](01-http-contract-204-and-headers.md) | HTTP Contract — 204 Semantics and Response Headers | high | M | correctness |
| [02](02-panic-abort-and-unwraps.md) | `panic = 'abort'` + Pervasive `unwrap()` Crashes the Whole Process | high | M | reliability |
| [03](03-upstream-timeouts-retries-buffering.md) | Upstream HTTP — Timeouts, Retry Semantics, Memory Buffering | high | M | reliability |
| [04](04-database-migrations-pool-schema.md) | Database — Broken Fresh-Install Migration, Schema Hazards, Pool Duplication | high | M | correctness |
| [05](05-cache-consistency-and-races.md) | Cache Consistency — Temp-Copy Lifecycle, Eviction Races, update_cache | medium | M | reliability |
| [06](06-security.md) | Security — API-Key Comparison, Unauthenticated Metrics, Secrets Handling | medium | S | security |
| [07](07-ci-docker-delivery.md) | Delivery — CI Gates, Docker Image Quality, Graceful Shutdown | medium | M | delivery |
| [08](08-observability.md) | Observability — Error Context, Cache Metrics, Log Configuration | medium | S | observability |
| [09](09-maintainability-dead-code-tests.md) | Maintainability — Dead Code, Duplication, Stale Docs, Test Coverage | low | M | maintainability |
| [10](10-performance-cpu-ram.md) | Performance — Sequential Cache Warming, Minor Hot-Path Costs | medium | M | performance |

## Consumer-contract verdict (asked explicitly)

- **`x-filename-b64` / `x-caption-b64` on 200:** guaranteed today. The only 200-with-body path of `GET /download/...` (`src/views.rs:107-122`) unconditionally appends both headers from a fully-populated `DownloadResult`. Risk is future regression (no test pins this) and a 500-instead-of-200 edge from unsanitized `Content-Disposition` — see Spec 01.2/01.3.
- **204 semantics:** NOT limited to "no file". 204 is also returned for every upstream failure (library/downloader/files-server errors, decode failures) — Spec 01.1. This actively violates book_bot's interpretation.

## Suggested order of work

1. Spec 02 (stop process-wide crashes) and Spec 04.1 (fresh installs are broken).
2. Spec 01 (consumer-facing 204/5xx contract).
3. Spec 03 (timeouts — currently unbounded hangs), Spec 04 remainder.
4. Specs 05–08.
5. Spec 09 opportunistically alongside the above.
