# Spec 10: Performance — sequential cache warming, minor hot-path costs

- **Priority:** medium
- **Effort:** M
- **Category:** performance

Specs 03–05 already cover the highest-impact resource issues (missing timeouts, full-file buffering with per-retry copies, duplicate DB pools, 300s acquire timeout, redundant indexes, update_cache races). This spec covers what remains once those land.

## Problem(s)

### 10.1 `start_update_cache` processes books strictly sequentially

`src/services/mod.rs:331-357`: the warm-up loop awaits `cache_file(...)` for one `(book, type)` pair at a time. Each call is a full download-from-downloader + upload-to-Telegram round trip (commonly seconds); 100 new books × 3 types × ~10s ≈ 50 minutes per scan, during which the scan holds its (post-Spec-05.3) single-flight slot and delivers no parallel utilization of network or upstream capacity.

**Fix:** Run `cache_file` calls through `futures::stream::iter(pairs).for_each_concurrent(N, ...)` with N=3–5 (env-tunable). Constraints:
- Requires Spec 04.3's `ON CONFLICT` insert first — concurrency multiplies the existing SELECT-then-INSERT race.
- Respect Telegram-side limits: N should stay below the bot pool size; on a 429 from the files server, back off the whole stream, not just one task.
- Also stop accumulating the entire scan result in memory before processing: `src/services/mod.rs:279-330` extends one big `Vec` across all pages; process page-by-page (fetch next page while the current one caches) so RAM is bounded by page size, not by three days of catalog.

### 10.2 Metadata subtasks are `spawn`-ed instead of joined (minor)

`src/services/mod.rs:195-206`: `download_from_telegram_files`, `get_filename`, and `get_book` are wrapped in three `tokio::task::spawn` + `.await.unwrap()` — three heap-allocated tasks and three panic-propagation hazards (`unwrap` on `JoinError`, see Spec 02) to express "run these concurrently".

**Fix:** `let (response, filename, book) = tokio::join!(...)` — same concurrency, no allocations, no `JoinError` unwraps. Zero behavior change otherwise.

### 10.3 `SELECT *` in hot queries (minor)

`src/services/mod.rs:61-72,178-191,333-341`, `src/views.rs:137-148`: all queries select every column. Today the table is 6 narrow columns so the cost is negligible — but Spec 05.4 adds `created_at`/`last_verified_at`, and `RETURNING *` / `SELECT *` with `query_as!` will silently start shipping them on every cache hit, and any future large column (e.g. a caption/blob) would too.

**Fix:** Enumerate columns explicitly in the `query_as!` calls when touching them for Spec 04/05 work. Not worth a standalone PR.

### 10.4 Config clone per request in middleware (minor)

`src/views.rs` auth middleware clones config strings per request (API key comparison path — see also Spec 06.1's constant-time comparison). Individually trivial; fix opportunistically by borrowing or wrapping config in `Arc` while implementing Spec 06.

## Acceptance criteria

- With a mocked downloader/files-server (~200ms per pair), warming 60 pairs at N=4 completes ≥3× faster than the sequential baseline; a mocked 429 pauses all in-flight warmers.
- Scan memory is bounded by one page of results regardless of the scan window (verified by code inspection).
- No `tokio::task::spawn` + `.await.unwrap()` triples remain in `download_from_cache`; replaced with `tokio::join!`.
- New/touched queries enumerate columns explicitly.
