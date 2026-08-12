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

## 05.5 Cross-normalized reuse race notes

`cache_file` (`src/services/mod.rs`) can satisfy a cache miss for
`(object_id, object_type, is_normalized)` by reusing the already-cached Telegram bytes
of the opposite `is_normalized` variant (`src/services/reuse.rs`) instead of always
cold-rebuilding from the external source mirror. This introduces the following races,
none of which require additional locking beyond what already exists:

- **R1 — opposite-normalized misses don't coalesce.** `CACHE_FILE_INFLIGHT`
  (`src/services/mod.rs`) is keyed by `(object_id, object_type, is_normalized)`, so a
  simultaneous cache miss for `is_normalized=true` and `is_normalized=false` on the same
  `(object_id, object_type)` are different single-flight keys and never coalesce; both
  cold-build independently. This is no worse than pre-existing behavior — it just means
  the reuse optimization is missed in that narrow window, not an error or a deadlock.
- **R2 — no recursive cold-build chains.** The reuse path
  (`try_reuse_from_opposite_normalized` / `reuse_from_source`) never calls `cache_file`
  or `get_cached_file_or_cache`; it only reads an already-committed row via
  `CachedFileRepository::find_by_object_id_object_type_is_normalized`. This structurally
  prevents a reuse attempt from triggering another reuse attempt or a recursive
  cold-build chain.
- **R3 — a gone source message falls back cleanly, without evicting.** If the source
  row's underlying Telegram message has been deleted (fetch returns 404/410/204 or
  otherwise fails), the reuse path returns `ReuseOutcome::NotEligible` and falls through
  to the normal cold-rebuild path. It never evicts the source row — eviction of a stale
  row remains solely the responsibility of `download_from_cache`.
- **R4 — concurrent insert of the target row is still resolved by `ON CONFLICT`.** If
  another path inserts the target `(object_id, object_type, is_normalized)` row
  concurrently before the reuse path's upsert commits, `cache_file`'s
  `INSERT ... ON CONFLICT (...) DO UPDATE` overwrites it with the reuse path's result.
  This is pre-existing `cache_file` behavior and is unchanged by this feature.
- **R5 — the source row structurally matches the opposite `is_normalized`.** The lookup
  filters `is_normalized = !is_normalized` in SQL, and the table's unique constraint on
  `(object_id, object_type, is_normalized)` guarantees at most one matching row, so the
  source row used for reuse is always exactly the opposite variant — no ambiguity.
