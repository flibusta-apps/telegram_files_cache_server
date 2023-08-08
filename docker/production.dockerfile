FROM rust:bullseye AS builder

WORKDIR /app

COPY . .

RUN cargo build --release --bin telegram_files_cache_server


FROM debian:bullseye-slim

RUN apt-get update \
    && apt-get install -y openssl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN update-ca-certificates

WORKDIR /app

COPY --from=builder /app/target/release/telegram_files_cache_server /usr/local/bin
ENTRYPOINT ["/usr/local/bin/telegram_files_cache_server"]
