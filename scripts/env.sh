#! /usr/bin/env sh

# Print all relevant env vars in KEY='VALUE' format for .env generation.
# Variables must be set externally (Docker env, docker-compose, CI secrets, etc.)

for var in API_KEY \
           POSTGRES_USER POSTGRES_PASSWORD POSTGRES_HOST POSTGRES_PORT POSTGRES_DB \
           DOWNLOADER_API_KEY DOWNLOADER_URL \
           LIBRARY_API_KEY LIBRARY_URL \
           FILES_SERVER_API_KEY FILES_SERVER_URL \
           BOT_TOKENS TEMP_CHANNEL_ID \
           SENTRY_DSN; do
  val=$(eval echo "\"\$$var\"")
  if [ -n "$val" ]; then
    echo "$var='$val'"
  fi
done
