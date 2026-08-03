use once_cell::sync::Lazy;

pub struct Config {
    pub api_key: String,

    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_host: String,
    pub postgres_port: u32,
    pub postgres_db: String,

    pub db_max_connections: u32,
    pub db_acquire_timeout_secs: u64,

    pub cache_warmup_concurrency: usize,

    pub downloader_api_key: String,
    pub downloader_url: String,

    pub library_api_key: String,
    pub library_url: String,

    pub files_api_key: String,
    pub files_url: String,

    pub bot_tokens: Vec<String>,
    pub temp_channel_id: i64,

    pub sentry_dsn: Option<String>,
}

fn get_env(env: &'static str) -> String {
    std::env::var(env).unwrap_or_else(|_| panic!("Cannot get the {} env variable", env))
}

impl Config {
    pub fn load() -> Config {
        Config {
            api_key: get_env("API_KEY"),

            postgres_user: get_env("POSTGRES_USER"),
            postgres_password: get_env("POSTGRES_PASSWORD"),
            postgres_host: get_env("POSTGRES_HOST"),
            postgres_port: get_env("POSTGRES_PORT")
                .parse()
                .expect("POSTGRES_PORT must be a valid u32"),
            postgres_db: get_env("POSTGRES_DB"),

            db_max_connections: std::env::var("DB_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            db_acquire_timeout_secs: std::env::var("DB_ACQUIRE_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),

            cache_warmup_concurrency: std::env::var("CACHE_WARMUP_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),

            downloader_api_key: get_env("DOWNLOADER_API_KEY"),
            downloader_url: get_env("DOWNLOADER_URL"),

            library_api_key: get_env("LIBRARY_API_KEY"),
            library_url: get_env("LIBRARY_URL"),

            files_api_key: get_env("FILES_SERVER_API_KEY"),
            files_url: get_env("FILES_SERVER_URL"),

            bot_tokens: serde_json::from_str(&get_env("BOT_TOKENS"))
                .expect("BOT_TOKENS must be a valid JSON array of strings"),
            temp_channel_id: get_env("TEMP_CHANNEL_ID")
                .parse()
                .expect("TEMP_CHANNEL_ID must be a valid i64"),

            sentry_dsn: std::env::var("SENTRY_DSN").ok(),
        }
    }
}

pub static CONFIG: Lazy<Config> = Lazy::new(Config::load);
