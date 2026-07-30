# Spec 07: Delivery — CI Gates, Docker Image Quality, Graceful Shutdown

- **Priority:** medium
- **Effort:** M
- **Category:** delivery

## Problem(s)

### 07.1 Nothing in CI blocks a broken build from deploying
- `.github/workflows/build_docker_image.yml` builds and pushes `:latest` + triggers the deploy webhook (lines 30-45) on every push to `main`, with **no test, clippy, or fmt gate** before it.
- `.github/workflows/rust-clippy.yml:49` runs clippy with `continue-on-error: true`, so lint failures never fail the workflow — it is advisory-only SARIF upload.
- `cargo test` is never run anywhere in CI, even though `tests/query_parsing.rs` exists.

A compile-warning-riddled or test-failing commit deploys straight to production.

**Fix:** Add a CI job (`cargo fmt --check`, `cargo clippy -- -D warnings` with `SQLX_OFFLINE=true`, `cargo test`) and make the docker build job depend on it (`needs:`). Remove `continue-on-error` or keep SARIF upload as a separate non-blocking step.

### 07.2 Dockerfile: no `.dockerignore`, no layer caching, no HEALTHCHECK, root user
`docker/production.dockerfile`:
- Line 5 `COPY . .` with **no `.dockerignore`** in the repo: local builds copy `target/` (huge), `.git`, and the developer's `.env` (secrets, Spec 06.5) into the build context/builder image.
- No dependency-layer caching (`COPY Cargo.toml Cargo.lock` + dummy build, or `cargo-chef`): every build recompiles all dependencies from scratch.
- `curl` is installed (line 13) but there is no `HEALTHCHECK` instruction, despite the app exposing `GET /health` (`src/views.rs:214`).
- No `USER` (Spec 06.6). No pinned base image digest; `rust:bookworm` floats.
- `SQLX_OFFLINE=true` is not set for the build; it works only because `DATABASE_URL` is absent — fragile and worth making explicit.

**Fix:** Add `.dockerignore` (`target/`, `.git/`, `.env`, `docs/`), cargo-chef or manifest-first layering, `ENV SQLX_OFFLINE=true`, `HEALTHCHECK CMD curl -sf http://localhost:8080/health || exit 1`, and a non-root `USER`.

### 07.3 No graceful shutdown — deploys kill in-flight downloads
`src/main.rs:57`: `axum::serve(listener, app).await.unwrap()` without `.with_graceful_shutdown(...)`. On SIGTERM (every deploy via the webhook flow) the process dies immediately, aborting active file streams to book_bot and in-progress cache uploads (leaving orphaned Telegram messages, cf. Spec 05.2).

**Fix:** Wire `with_graceful_shutdown` on SIGTERM/SIGINT with a drain window; ensure the deploy infrastructure sends SIGTERM and honors a stop grace period.

### 07.4 Health endpoint does not reflect readiness
`src/views.rs:162-164` returns 200 unconditionally. It reports "alive" even if the DB pool is unusable, so an orchestrator health/readiness probe (once added per 07.2) cannot detect a broken instance.

**Fix:** Keep `/health` as liveness; add `/ready` that performs `SELECT 1` on the pool.

### 07.5 `actions-rs/toolchain` is archived/unmaintained
`.github/workflows/rust-clippy.yml:34` uses `actions-rs/toolchain` (archived since 2023, incompatible warnings on current runners).

**Fix:** Replace with `dtolnay/rust-toolchain@stable`.

## Acceptance criteria
- Pushing a commit that fails `cargo test` or `cargo clippy -D warnings` does not produce/push a docker image or fire the deploy webhook.
- `docker build` succeeds from a clean checkout without a database, and a repo containing `.env` does not leak it into the image (verify with `docker history`/`dive` or a build-context listing).
- Image defines `HEALTHCHECK` and runs as non-root; `docker inspect` shows health status.
- Sending SIGTERM to the running container lets in-flight responses complete (bounded drain) before exit.
