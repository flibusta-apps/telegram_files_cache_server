use once_cell::sync::Lazy;

pub struct Config {
    pub api_key: String,

    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_host: String,
    pub postgres_port: u32,
    pub postgres_db: String,

    pub downloader_api_key: String,
    pub downloader_url: String,

    pub library_api_key: String,
    pub library_url: String,

    pub files_api_key: String,
    pub files_url: String,

    pub bot_tokens: Vec<String>,
    pub temp_channel_id: i64,

    pub sentry_dsn: String,
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
            postgres_port: get_env("POSTGRES_PORT").parse().unwrap(),
            postgres_db: get_env("POSTGRES_DB"),

            downloader_api_key: get_env("DOWNLOADER_API_KEY"),
            downloader_url: get_env("DOWNLOADER_URL"),

            library_api_key: get_env("LIBRARY_API_KEY"),
            library_url: get_env("LIBRARY_URL"),

            files_api_key: get_env("FILES_SERVER_API_KEY"),
            files_url: get_env("FILES_SERVER_URL"),

            bot_tokens: serde_json::from_str(&get_env("BOT_TOKENS")).unwrap(),
            temp_channel_id: get_env("TEMP_CHANNEL_ID").parse().unwrap(),

            sentry_dsn: get_env("SENTRY_DSN"),
        }
    }
}

pub static CONFIG: Lazy<Config> = Lazy::new(Config::load);
