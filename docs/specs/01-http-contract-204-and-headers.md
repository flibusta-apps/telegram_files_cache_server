# Spec 01: HTTP Contract — 204 Semantics and Response Headers

- **Priority:** high
- **Effort:** M
- **Category:** correctness

## Problem(s)

### 01.1 204 conflates "file does not exist" with "upstream failure"
`book_bot` treats 204 as "no file exists" (`book_bot/src/bots/approved_bot/services/book_cache/mod.rs:108-110,169-171`). The cache server, however, returns 204 for *every* failure mode:

- `src/views.rs:47,76,87,92` — every `None` from `get_cached_file_or_cache` / `download_from_cache` becomes `StatusCode::NO_CONTENT`.
- `src/services/mod.rs:140-175` (`cache_file`) — returns `None` (→204) when the library API errors (line 143), when the downloader errors (line 161), or when the upload to telegram_files_server errors (line 172). Only line 158 (`downloader` returned HTTP 204) is a genuine "file does not exist".
- `src/services/mod.rs:241-255` (`download_from_cache`) — filename lookup or book lookup failure also returns `None` → 204.

A transient outage of book_library_server, books_downloader, or telegram_files_server is thus reported to users as "book has no file", and book_bot will not retry. This silently corrupts the consumer-facing behavior.

**Fix:** Make `cache_file`/`download_from_cache` return `Result<Option<...>, Error>` distinguishing "definitively no file" (`Ok(None)`) from "upstream failure" (`Err`). Map `Err` to `502 Bad Gateway` (or `503`) in `src/views.rs`; keep 204 only for the genuine downloader-204 case and for `DELETE` misses.

### 01.2 `x-filename-b64` / `x-caption-b64` are guaranteed on 200 — keep it that way (contract test missing)
Verified: the only 200-with-body path of `GET /download/{id}/{type}/` is `src/views.rs:107-122`, which unconditionally appends both headers built from `DownloadResult` (`src/services/mod.rs:263-268` always populates `filename` and `caption`; base64 STANDARD output is always a valid header value). So the book_bot contract (it `unwrap()`s both headers on 200) currently holds structurally. However, nothing protects this contract: there is no test asserting the headers, and any refactor of `download_cached_file` can silently break book_bot (which panics on a missing header).

**Fix:** Add an integration/handler test that asserts every 200 response from the download route carries valid base64 `x-filename-b64` and `x-caption-b64` headers. Document the contract in `AGENTS.md`.

### 01.3 Unsanitized `filename_ascii` in `Content-Disposition` can turn a good response into a 500
`src/views.rs:108-111`:
```rust
(header::CONTENT_DISPOSITION, format!("attachment; filename={filename_ascii}"))
```
`filename_ascii` comes verbatim from the downloader service (`src/services/downloader/mod.rs:46-83`). It is not quoted or sanitized; if it ever contains a non-ASCII byte or control character, `AppendHeaders`' `TryInto<HeaderValue>` fails and axum replaces the whole response with a 500. If it contains spaces or `;`, the header is valid but semantically broken for HTTP clients.

**Fix:** Quote and sanitize: strip non-visible-ASCII characters, escape `"`, and emit `attachment; filename="<sanitized>"`. Fall back to a constant name (e.g. `file.bin`) if empty after sanitization.

### 01.4 Download success is not verified before committing to a 200
`src/views.rs:103-105` streams `data.response` (a still-open reqwest body from telegram_files_server). If the upstream stream dies mid-transfer, the client has already received the 200 + headers and gets a truncated body with no error status. There is no Content-Length forwarded either, so consumers cannot detect truncation.

**Fix:** Forward upstream `Content-Length` on the response when available so consumers can detect truncated transfers.

## Acceptance criteria
- Upstream errors (library/downloader/files-server 5xx, timeouts, decode failures) produce 5xx from the cache server, not 204; downloader-204 still produces 204.
- A test exercises the 200 path of `GET /download/...` and asserts both `x-filename-b64` and `x-caption-b64` are present and base64-decodable.
- `Content-Disposition` is well-formed (quoted, ASCII-only) for filenames containing spaces, quotes, semicolons, and non-ASCII bytes; no 500 is produced by header construction.
- `Content-Length` is forwarded on streamed downloads when the upstream provides it.
