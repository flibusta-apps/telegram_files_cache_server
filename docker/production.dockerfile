FROM rust:bookworm@sha256:77fac8b98f9f46062bb680b6d25d5bcaabfc400143952ebc572e924bcbedc3fa AS builder

WORKDIR /app

ENV SQLX_OFFLINE=true

COPY Cargo.toml Cargo.lock ./

RUN mkdir -p src \
  && echo "fn main() {}" > src/main.rs \
  && cargo build --release --bin telegram_files_cache_server \
  && rm -rf src

COPY . .

RUN touch src/main.rs && cargo build --release --bin telegram_files_cache_server


FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

RUN apt-get update \
  && apt-get install -y openssl ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

RUN update-ca-certificates

RUN useradd --system --create-home --shell /usr/sbin/nologin app

COPY ./scripts/*.sh /
RUN chmod +x /*.sh

WORKDIR /app
RUN chown app:app /app

COPY --from=builder /app/target/release/telegram_files_cache_server /usr/local/bin

USER app

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 CMD curl -sf http://localhost:8080/health || exit 1

CMD ["/start.sh"]
