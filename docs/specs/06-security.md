# Spec 06: Security — API-Key Comparison, Unauthenticated Metrics, Secrets Handling

- **Priority:** medium
- **Effort:** S
- **Category:** security

## Problem(s)

### 06.1 Non-constant-time API key comparison
`src/views.rs:180`:
```rust
if auth_header != CONFIG.api_key {
```
Plain string comparison short-circuits on the first differing byte, enabling (low-practicality, but free-to-fix) timing side channels on the single static API key that guards all routes.

**Fix:** Compare with a constant-time function (e.g. `subtle::ConstantTimeEq` on byte slices, after length check folded into the comparison).

### 06.2 `/metrics` is unauthenticated
`src/views.rs:211-212` exposes Prometheus metrics with no auth, while everything else under `/api/v1/` requires the API key (`src/views.rs:207`). `axum-prometheus` labels include route templates and status codes — useful reconnaissance (endpoints, traffic volume, error rates) for anyone who can reach the port. AGENTS.md line 31 confirms "no auth" as current, undocumented-risk behavior.

**Fix:** Require the API key (or a separate metrics token / network-level restriction) for `/metrics`, or bind metrics to a separate internal port.

### 06.3 Secrets are re-written to a file inside the container
`scripts/start.sh:3` (`/env.sh > ./.env`) serializes every secret (`API_KEY`, `POSTGRES_PASSWORD`, `BOT_TOKENS`, all upstream keys — `scripts/env.sh:6-17`) into `/app/.env` at container start, even though the process already receives them via environment (`dotenvy` at `src/main.rs:19` merely re-reads them). This widens exposure: any file-read primitive, misconfigured volume, or `docker cp`/image commit leaks all credentials at once.

**Fix:** Delete the `.env` generation entirely — `Config::load` reads the environment directly; `dotenv().ok()` already tolerates a missing file.

### 06.4 `SENTRY_DSN` is mandatory and unvalidated
`src/config.rs:54` requires `SENTRY_DSN` and `src/main.rs:22` `unwrap()`s the DSN parse. Local/dev runs without Sentry are impossible, which encourages copying the production DSN (a credential) into dev environments.

**Fix:** Make `SENTRY_DSN` optional (`ClientOptions { dsn: None }` disables sending); parse with a clear error message when present.

### 06.5 Live database credential in local working tree
`telegram_files_cache_server/.env` (gitignored — verified via `git check-ignore`) contains a real `DATABASE_URL` with a password to a publicly resolvable host (`kurbezz.me:54322`). Not committed, but it is a production-looking credential sitting in plaintext on a dev machine and used by sqlx CLI.

**Fix:** Rotate the credential if it is production; prefer a dev database for `cargo sqlx prepare`.

### 06.6 Container runs as root
`docker/production.dockerfile:10-24` has no `USER` directive; the service and `start.sh` run as root. See also Spec 07 for other Docker issues.

**Fix:** Add a non-root user (`USER app`) after installing packages.

## Acceptance criteria
- API key check uses a constant-time comparison.
- `/metrics` is not reachable without credentials (or is bound to an internal-only listener); `/health` remains open.
- Container filesystem contains no generated `.env`; process runs as non-root (verified with `docker exec whoami` / `ls /app/.env`).
- App starts with `SENTRY_DSN` unset.
