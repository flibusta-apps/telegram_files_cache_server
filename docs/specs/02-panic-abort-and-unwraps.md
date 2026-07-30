# Spec 02: `panic = 'abort'` + Pervasive `unwrap()` Crashes the Whole Process

- **Priority:** high
- **Effort:** M
- **Category:** reliability

## Problem(s)

### 02.1 Release profile aborts the process on any panic
`Cargo.toml:13`:
```toml
panic = 'abort'
```
With `panic = 'abort'`, Tokio's per-task panic isolation does not apply: any panic in any request handler or background task kills the entire server, dropping all in-flight downloads. Every `unwrap()` below is therefore a process-wide DoS vector, not a single failed request.

**Fix:** Either remove `panic = 'abort'` from the release profile, or (better) do both: keep unwinding and eliminate the `unwrap()`s on fallible runtime data listed below.

### 02.2 `unwrap()` on database queries in request handlers
- `src/views.rs:137-148` — `delete_cached_file`: `fetch_optional(&db).await.unwrap()`.
- `src/services/mod.rs:61-72` — `get_cached_file_or_cache`: `fetch_optional(&db).await.unwrap()`.
- `src/services/mod.rs:93-102` — DELETE inside `get_cached_file_copy`: `.execute(&db).await.unwrap()`.
- `src/services/mod.rs:178-191` — `cache_file` INSERT: `.fetch_one(&db).await.unwrap()`. This also panics on a unique-constraint violation (see Spec 04.3 race), i.e. two concurrent requests for the same uncached book can abort the server.

Any transient Postgres error (connection drop, pool exhaustion after the 300s acquire timeout, constraint violation) aborts the process.

**Fix:** Return `Result` from these functions, convert DB errors to 500/502 responses in handlers, and log to Sentry.

### 02.3 `unwrap()` chain in `get_cached_file_copy` on Telegram data
`src/services/mod.rs:104-119`:
```rust
let new_original = get_cached_file_or_cache(...).await.unwrap();   // line 111
bot.copy_message(...).await.unwrap()                               // line 119
```
If re-caching fails (`None`, e.g. upstream down) or the second `copy_message` fails (bot rate-limited, message deleted), the process aborts. Also `original.message_id.try_into().unwrap()` (`src/services/mod.rs:87,116`) panics on any stored `message_id` that does not fit `i32`.

**Fix:** Propagate errors; respond 502 on re-cache/copy failure. Use a checked conversion with a logged error for `message_id`.

### 02.4 `JoinHandle::unwrap()` on spawned tasks
`src/services/mod.rs:207,241,249` — `response_task.await.unwrap()` etc. A panic inside a spawned task becomes a `JoinError` and this `unwrap()` re-panics in the handler (moot under `abort`, but still wrong once unwinding is restored).

**Fix:** Handle `JoinError` as an internal error path.

### 02.5 Startup unwraps with poor diagnostics
- `src/main.rs:22` — `Dsn::from_str(&CONFIG.sentry_dsn).unwrap()`: an invalid/empty `SENTRY_DSN` crashes startup with an opaque panic (see also Spec 06.4).
- `src/config.rs:39,51,52` — `parse().unwrap()` / `serde_json::from_str(...).unwrap()` for `POSTGRES_PORT`, `BOT_TOKENS`, `TEMP_CHANNEL_ID` panic without naming the offending variable.
- `src/services/download_utils.rs:51` — `tmp_file.seek(...).unwrap()` (currently dead code, see Spec 09.1).

**Fix:** Validate config with named error messages (e.g. `expect("TEMP_CHANNEL_ID must be an integer")` or a `Result`-based loader).

## Acceptance criteria
- `grep -rn "unwrap()" src/` shows no `unwrap()` on runtime-fallible values (DB queries, HTTP responses, Telegram API calls, numeric conversions of stored data); startup config errors name the variable.
- Killing Postgres or telegram_files_server while serving requests yields 5xx responses, and the process stays alive (verified manually or by test).
- Concurrent duplicate cache requests do not abort the process.
