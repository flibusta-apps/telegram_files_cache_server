# Spec 03: Upstream HTTP — Timeouts, Retry Semantics, Memory Buffering

- **Priority:** high
- **Effort:** M
- **Category:** reliability

## Problem(s)

### 03.1 No timeouts on any upstream HTTP client
Three separate clients are all built with defaults, which have **no total timeout**:
- `src/services/telegram_files/mod.rs:14` — `Lazy::new(reqwest::Client::new)`
- `src/services/downloader/mod.rs:8` — same
- `src/services/book_library/mod.rs:12` — same

A hung upstream (accepted TCP, no response) blocks the handler forever; the retry helper's `error.is_timeout()` branch (`src/services/retry.rs:15-17`) can never fire because no timeout is configured. Combined with book_bot waiting on this server, one stuck upstream call pins client requests indefinitely.

**Fix:** Build clients with `connect_timeout` (e.g. 5s) and a request timeout appropriate per service (short for `book_library`/`filename`, longer or body-read-based for file downloads/uploads). Consider one shared client with per-request `.timeout()`.

### 03.2 Retry of non-idempotent upload can create duplicate Telegram messages
`src/services/telegram_files/mod.rs:96-140` retries the multipart POST `/api/v1/files/upload/` via `retry_transient`. `is_transient_error` treats `error.is_request()` and 5xx as retryable (`src/services/retry.rs:19-31`). If the first upload actually succeeded server-side but the response was lost (connection reset, gateway 502), the retry uploads the file again — orphaning a Telegram message in the channel that nothing tracks or deletes.

**Fix:** Either make upload idempotent (dedup key on the files server), or restrict upload retries to connect-phase errors only, and log the potential-duplicate case.

### 03.3 Whole file buffered in memory, then cloned per retry attempt
`src/services/telegram_files/mod.rs:86-89` reads the entire downloader response into `body_bytes`, and line 105 (`Part::bytes(body_bytes.to_vec())`) makes a full extra copy on every attempt. For large books and concurrent cache-warm runs (`start_update_cache` iterates every new book, `src/services/mod.rs:331-357`), memory usage is unbounded (N concurrent uploads × 2 × file size). Note `response_to_tempfile` (`src/services/download_utils.rs:24-55`) already implements spooled-tempfile buffering but is unused.

**Fix:** Buffer to a spooled temp file (reuse `response_to_tempfile`) and stream the multipart part from it (`Body::wrap_stream`/`Part::stream_with_length`), or at minimum use `Part::stream` from `Bytes` without `to_vec()` (Bytes is cheaply cloneable).

### 03.4 Retry backoff has no jitter and retries inside a user-facing request path
`src/services/retry.rs:56-80`: fixed exponential backoff `2^attempt` seconds (2s, 4s), no jitter, `MAX_RETRIES=3`. All retries run inline inside HTTP handlers, so a struggling upstream turns each client request into a ~6+ second stall and synchronized retry storms across concurrent requests.

**Fix:** Add jitter, cap total retry budget for interactive routes (e.g. 1 retry for the download path), keep 3 for background cache warming.

### 03.5 `download_from_cache` misuses status check after `error_for_status`
`src/services/mod.rs:207-224`: `download_from_telegram_files` already applies `.error_for_status()` (`src/services/telegram_files/mod.rs:44`), so at line 209 `v.status() != 200` matches only non-error non-200 codes (e.g. 204). The cached row is then deleted (lines 210-221) — treating a possibly-empty/odd success reply as "stale cache". Meanwhile a genuine 404/410 ("message deleted") arrives as `Err` and *also* deletes the row (lines 225-238) — together with every transient 5xx/timeout, which wrongly evicts valid cache entries during upstream outages.

**Fix:** Distinguish status classes: evict cache only on 404/410-style "message no longer exists" responses; treat 5xx/timeouts as retryable without eviction; handle 204 explicitly.

## Acceptance criteria
- All reqwest clients have explicit connect and request timeouts; a blackholed upstream produces a 5xx within the configured budget.
- Upload retries cannot silently duplicate Telegram messages (idempotency key or connect-error-only retry, with a test or documented rationale).
- Caching a large file does not hold more than one copy of it in RAM (verified by code inspection: no `bytes().await` of full body followed by `to_vec()`).
- Cache rows are evicted only on definitive "gone" upstream answers, not on 5xx/timeout.
