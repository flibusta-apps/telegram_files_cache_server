use once_cell::sync::Lazy;

pub struct Config {
    pub api_key: String,

    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_host: String,
    pub postgres_port: u16,
    pub postgres_db: String,

    pub db_max_connections: u32,
    pub db_acquire_timeout_secs: u64,

    pub cache_warmup_concurrency: usize,

    pub upload_timeout_secs: u64,

    pub cross_normalized_reuse: bool,

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

fn get_secret(env: &'static str) -> String {
    let v = get_env(env);
    if v.trim().is_empty() {
        panic!("The {} env variable must not be empty", env);
    }
    v
}

/// Optional numeric env var: unset → default; set but unparsable → startup failure.
fn get_parsed_or<T: std::str::FromStr>(env: &'static str, default: T) -> T {
    match std::env::var(env) {
        Err(_) => default,
        Ok(v) => v
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("The {} env variable is not a valid number: {:?}", env, v)),
    }
}

impl Config {
    pub fn load() -> Config {
        let config = Config {
            api_key: get_secret("API_KEY"),

            postgres_user: get_env("POSTGRES_USER"),
            postgres_password: get_env("POSTGRES_PASSWORD"),
            postgres_host: get_env("POSTGRES_HOST"),
            postgres_port: get_env("POSTGRES_PORT")
                .parse()
                .expect("POSTGRES_PORT must be a valid u16"),
            postgres_db: get_env("POSTGRES_DB"),

            db_max_connections: get_parsed_or("DB_MAX_CONNECTIONS", 10),
            db_acquire_timeout_secs: get_parsed_or("DB_ACQUIRE_TIMEOUT_SECS", 5),

            cache_warmup_concurrency: get_parsed_or("CACHE_WARMUP_CONCURRENCY", 4),

            upload_timeout_secs: get_parsed_or("UPLOAD_TIMEOUT_SECS", 600),

            cross_normalized_reuse: get_parsed_or("CROSS_NORMALIZED_REUSE", true),

            downloader_api_key: get_secret("DOWNLOADER_API_KEY"),
            downloader_url: get_env("DOWNLOADER_URL"),

            library_api_key: get_secret("LIBRARY_API_KEY"),
            library_url: get_env("LIBRARY_URL"),

            files_api_key: get_secret("FILES_SERVER_API_KEY"),
            files_url: get_env("FILES_SERVER_URL"),

            bot_tokens: serde_json::from_str(&get_env("BOT_TOKENS"))
                .expect("BOT_TOKENS must be a valid JSON array of strings"),
            temp_channel_id: get_env("TEMP_CHANNEL_ID")
                .parse()
                .expect("TEMP_CHANNEL_ID must be a valid i64"),

            sentry_dsn: std::env::var("SENTRY_DSN").ok(),
        };
        config.validate();
        config
    }

    fn validate(&self) {
        assert!(!self.bot_tokens.is_empty(), "BOT_TOKENS must not be empty");
        assert!(
            self.cache_warmup_concurrency > 0,
            "CACHE_WARMUP_CONCURRENCY must be > 0"
        );
        assert!(
            self.db_max_connections > 0,
            "DB_MAX_CONNECTIONS must be > 0"
        );
        assert!(
            self.db_acquire_timeout_secs > 0,
            "DB_ACQUIRE_TIMEOUT_SECS must be > 0"
        );
        assert!(
            self.upload_timeout_secs > 0,
            "UPLOAD_TIMEOUT_SECS must be > 0"
        );
    }
}

pub static CONFIG: Lazy<Config> = Lazy::new(Config::load);
