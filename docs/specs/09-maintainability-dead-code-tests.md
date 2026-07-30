# Spec 09: Maintainability — Dead Code, Duplication, Stale Docs, Test Coverage

- **Priority:** low
- **Effort:** M
- **Category:** maintainability

## Problem(s)

### 09.1 Dead code and unused dependencies
Verified unreferenced outside their definitions:
- `src/services/mod.rs:271-277` — `struct FileLinkResult` (never constructed).
- `src/services/download_utils.rs:24-55` — `response_to_tempfile` (never called; ironically it is the fix for Spec 03.3).
- `src/services/book_library/mod.rs:61-63` — `get_sources` (never called).
- `src/services/book_library/types.rs:18-26,46-58` — `struct Book` and `BookWithRemote::from_book` (never used).
- `Cargo.toml:47-48` — `futures-core` and `async-stream` are not referenced anywhere in `src/` (`grep` confirms no `async_stream`/`futures_core` usage).

**Fix:** Delete the dead items and dependencies (or wire `response_to_tempfile` into the upload path per Spec 03.3).

### 09.2 SQL and HTTP-client duplication
- The DELETE query exists three times: `src/views.rs:137-148`, `src/services/mod.rs:93-102`, `src/repository.rs:18-31`; the SELECT twice: `src/services/mod.rs:61-72` and `333-341`. `CachedFileRepository` exists (`src/repository.rs:3`) but is used only in one of five call sites.
- Three identical `pub static CLIENT: Lazy<reqwest::Client>` statics: `src/services/telegram_files/mod.rs:14`, `src/services/downloader/mod.rs:8`, `src/services/book_library/mod.rs:12` — three connection pools where one shared, timeout-configured client would do (see Spec 03.1).
- Repeated `.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)` boilerplate throughout the services (`src/services/telegram_files/mod.rs`, `downloader/mod.rs`, `book_library/mod.rs`) — a typed error enum (`thiserror`) would remove ~30 lines and enable the status-code discrimination required by Specs 01.1 and 03.5.

**Fix:** Route all `cached_files` access through `CachedFileRepository`; consolidate to one configured `reqwest::Client`; introduce a service error enum.

### 09.3 Stale/incorrect AGENTS.md and empty `examples/`
- `AGENTS.md:19` — "No tests exist in this repo." — false: `tests/query_parsing.rs` exists with 12 tests.
- `AGENTS.md:27` documents "`?copy` to get a temp Telegram copy", but `copy` is a **required** boolean query parameter (`src/views.rs:32-33` — no `#[serde(default)]`), so `GET /{id}/{type}/` without `copy=` returns 400.
- `examples/` is an empty committed directory (nothing to run).

**Fix:** Correct AGENTS.md (tests exist and how to run them; `copy` is required or make it optional-default-false in code); remove the empty `examples/` dir or add a real example.

### 09.4 Test coverage is near zero and the one test duplicates production types
`tests/query_parsing.rs:1-24` re-declares mirror copies of the query structs ("Kept local so the test stays a pure unit test"), so a change to the real structs in `src/views.rs:31-36,59-63,125-129` would not fail the test — it validates serde behavior, not the application. Nothing covers: handler status mapping (204/200/5xx — the book_bot contract, Spec 01.2), header emission, `get_caption` truncation logic (`src/services/book_library/types.rs:84-106`, has a subtle `< 1024` byte-length budget worth pinning), retry classification (`src/services/retry.rs:8-48`), or repository queries.

**Fix:** Import the real query structs (make them `pub`, they already are — the crate is a lib per `src/main.rs:1-6`... note: it is a binary; add a `lib.rs` or `#[path]` include so tests can reach `views`), add router-level tests with `tower::ServiceExt::oneshot` and a mocked service layer, and unit tests for `get_caption` and `is_transient_error`.

## Acceptance criteria
- `cargo +nightly udeps` (or manual grep) shows no unused dependencies; no dead pub items remain (or are `#[allow(dead_code)]`-annotated with a reason).
- Exactly one definition each of the cached-file SELECT/DELETE SQL and one shared HTTP client.
- AGENTS.md accurately describes tests and the `copy` parameter; `examples/` is gone or populated.
- `cargo test` covers query structs from the real crate, `get_caption`, `is_transient_error`, and at least one router-level test asserting the 200-headers and 204 contract.
