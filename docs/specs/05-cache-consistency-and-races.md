# Spec 05: Cache Consistency — Temp-Copy Lifecycle, Eviction Races, update_cache

- **Priority:** medium
- **Effort:** M
- **Category:** reliability

## Problem(s)

### 05.1 `TEMP_MESSAGES` keyed by cached-file id: concurrent copies delete each other
`src/services/mod.rs:123`:
```rust
TEMP_MESSAGES.insert(original.id, message_id).await;
```
The moka cache (`src/services/mod.rs:37-53`) is keyed by `cached_files.id`. Two concurrent `?copy=true` requests for the same file each create a Telegram copy; the second `insert` *replaces* the first entry, firing the eviction listener which deletes the first temp message — potentially while the first consumer is still forwarding it. Additionally `time_to_idle(16s)` means any consumer slower than 16 seconds gets its message deleted mid-use, silently.

**Fix:** Key by the temp `message_id` (unique per copy) so entries never collide; use `time_to_live` sized to consumer behavior and document the TTL in the API contract. Consider skipping deletion errors logging (`let _ =` at line 44 hides failures — orphaned temp messages accumulate silently).

### 05.2 `DELETE` endpoint and evictions leave orphaned Telegram messages
`src/views.rs:137-148` deletes only the DB row; the uploaded message in the storage channel is never removed. The same happens on cache eviction in `download_from_cache` (`src/services/mod.rs:210-238`) and on the re-cache path in `get_cached_file_copy` (`src/services/mod.rs:93-102`): the old (possibly still valid) message stays in Telegram forever. Storage channels grow unboundedly and there is no way to correlate orphans back.

**Fix:** Decide and document the ownership model. If the cache owns uploads, delete the Telegram message (via telegram_files_server or bot API) after removing the row, best-effort with logging.

### 05.3 `POST /update_cache` has no concurrency guard or status reporting
`src/views.rs:156-160`:
```rust
tokio::spawn(start_update_cache(db));
StatusCode::OK.into_response()
```
Every POST spawns a *new* full scan (3 days of books, all types — `src/services/mod.rs:279-357`). Repeated calls run concurrently: they race on the SELECT-then-INSERT (each task caches the same missing book → duplicate uploads → unique violation → panic per Spec 02.2/04.3). There is also no way to observe progress, failure, or completion; errors vanish into logs (`log::error!` at line 327).

**Fix:** Guard with a `tokio::sync::Mutex`/`AtomicBool` "already running" flag returning `409 Conflict` (or coalescing), and expose last-run status via the endpoint or a metric.

### 05.4 Stale-metadata path (`copy=false`) never validates or heals
`src/views.rs:38-57` returns raw `message_id`/`chat_id` from the DB. The download path self-heals stale entries (`src/views.rs:79-95` deletes + re-caches), but the metadata path hands out possibly dead references with no validation, and there is no TTL/`created_at` column to age entries out (schema in `migrations/20260116094605_create_cached_files_table.sql:5-11` has no timestamp columns).

**Fix:** Add `created_at` (and optionally `last_verified_at`) columns; document that `copy=false` metadata may be stale; consumers should fall back to `?copy=true` or download on failure (book_bot already deletes-and-retries — verify and document this loop).

## Acceptance criteria
- Concurrent `?copy=true` requests for the same object each receive a temp message that survives its documented TTL.
- Deleting a cache entry (endpoint or eviction) has a defined, documented fate for the Telegram message; orphan-producing paths log what they orphan.
- Two rapid `POST /update_cache` calls result in a single scan (second call coalesced or rejected with 409).
- Cache rows carry `created_at`; staleness semantics of `copy=false` are documented.
