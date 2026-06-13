use std::env;
use std::time::Duration;

/// Runtime configuration, loaded once from the environment at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub http_host: String,
    pub http_port: u16,
    pub database_url: String,
    pub cors_allowed_origins: Vec<String>,
    pub session_ttl: Duration,
    pub heartbeat_timeout: Duration,
    pub max_players_per_world: i32,
    pub join_request_ttl: Duration,
    pub join_ticket_ttl: Duration,
    pub invite_ttl: Duration,
    pub search_min_query_length: usize,
    pub search_max_results: i64,
}

impl Config {
    /// Build configuration from environment variables, applying sane defaults
    /// for everything except `DATABASE_URL`, which is required.
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url =
            env::var("DATABASE_URL").map_err(|_| anyhow::anyhow!("DATABASE_URL must be set"))?;

        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|origin| origin.trim().to_string())
            .filter(|origin| !origin.is_empty())
            .collect();

        Ok(Self {
            http_host: env::var("HTTP_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            http_port: parse_env("HTTP_PORT", 8080),
            database_url,
            cors_allowed_origins,
            session_ttl: Duration::from_secs(parse_env("SESSION_TTL_SECONDS", 86_400)),
            heartbeat_timeout: Duration::from_secs(parse_env("HEARTBEAT_TIMEOUT_SECONDS", 60)),
            max_players_per_world: parse_env("MAX_PLAYERS_PER_WORLD", 5),
            join_request_ttl: Duration::from_secs(parse_env("JOIN_REQUEST_TTL_SECONDS", 60)),
            join_ticket_ttl: Duration::from_secs(parse_env("JOIN_TICKET_TTL_SECONDS", 60)),
            invite_ttl: Duration::from_secs(parse_env("INVITE_TTL_SECONDS", 600)),
            search_min_query_length: parse_env("SEARCH_MIN_QUERY_LENGTH", 2),
            search_max_results: parse_env("SEARCH_MAX_RESULTS", 20),
        })
    }
}

fn parse_env<T>(key: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    match env::var(key) {
        Ok(value) => value.parse().unwrap_or(default),
        Err(_) => default,
    }
}
