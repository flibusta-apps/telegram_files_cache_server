# AGENTS.md

## Project Overview

Rust async server that caches Telegram files (books) for fast retrieval. Single binary: `telegram_files_cache_server`.

**Stack**: Axum 0.8, Tokio, sqlx (PostgreSQL), teloxide (Telegram bot SDK), moka (in-memory cache), Sentry.

## Commands

```
cargo build --release --bin telegram_files_cache_server   # build
cargo run                                                 # run dev
cargo fmt                                                 # format
cargo clippy --all-features                               # lint
cargo check                                               # fast compile check
```

No tests exist in this repo.

Pre-commit runs: `fmt` → `cargo-check` → `clippy`.

## Architecture

- **Entry point**: `src/main.rs` — loads `.env`, inits Sentry, runs migrations, starts Axum on `0.0.0.0:8080`.
- **Routes** (under `/api/v1/`, all require `Authorization: <API_KEY>`):
  - `GET /{object_id}/{object_type}/` — get cached file metadata (may be stale; see code doc comment) or `?copy` to get a temp Telegram copy (expires from the temp channel after 5 minutes — consumer must forward/consume within that window)
  - `GET /download/{object_id}/{object_type}/` — stream file content
  - `DELETE /{object_id}/{object_type}/` — delete cache entry (also best-effort deletes the underlying Telegram message)
  - `POST /update_cache` — async cache warmup (fetches books from last 3 days); returns 200 with status JSON if started, 409 with last-run status JSON if a scan is already in progress (concurrent calls coalesce onto one scan)
  - `GET /metrics` — Prometheus metrics (no auth)
  - `GET /health` — health check (no auth)
- **Services**: `book_library` (external API), `downloader` (file download), `telegram_files` (upload/download via Telegram bots, round-robin).
- **Config**: `src/config.rs` — global `CONFIG` static loaded from env vars at startup via `dotenvy`.

## Required Environment Variables

`API_KEY`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_HOST`, `POSTGRES_PORT`, `POSTGRES_DB`,
`DOWNLOADER_API_KEY`, `DOWNLOADER_URL`, `LIBRARY_API_KEY`, `LIBRARY_URL`,
`FILES_SERVER_API_KEY`, `FILES_SERVER_URL`, `BOT_TOKENS` (JSON array), `TEMP_CHANNEL_ID`, `SENTRY_DSN`.

`.env` is gitignored. `DATABASE_URL` in `.env` is used by sqlx CLI only; the app builds the URL from individual `POSTGRES_*` vars.

**OPTIONAL** (not required, sensible defaults used if unset):
- `DB_MAX_CONNECTIONS` — max Postgres pool connections (default: `10`).
- `DB_ACQUIRE_TIMEOUT_SECS` — pool connection acquire timeout in seconds (default: `5`).

## SQLx

Uses **offline mode**. Compiled query metadata lives in `.sqlx/`. If you add/modify a `sqlx::query!` or `sqlx::query_as!`, run:

```
cargo sqlx prepare
```

Requires `DATABASE_URL` set and a live Postgres connection. Without this, compilation will fail.

## Migrations

Run automatically on startup via `sqlx::migrate!("./migrations")`. Migrations live in `migrations/`.

## Production

- Dockerfile: `docker/production.dockerfile` (multi-stage, builds release binary).
- CI pushes to `ghcr.io` on main push, then triggers deploy via webhook.
- Production env vars are injected directly (Docker env, docker-compose, CI secrets). `scripts/env.sh` → `.env`.
