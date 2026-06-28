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
    pub rate_limit: RateLimitConfig,
    pub request_log: RequestLogConfig,
    pub yggdrasil: YggdrasilConfig,
    pub textures: TexturesConfig,
    pub catalog: CatalogConfig,
    pub bundles: BundlesConfig,
    pub admin: AdminConfig,
}

/// Per-IP sliding-window rate limit for unauthenticated credential endpoints.
///
/// Env vars: `RATE_LIMIT_MAX_ATTEMPTS` (default 10),
/// `RATE_LIMIT_WINDOW_SECONDS` (default 60).
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub max_attempts: u32,
    pub window: Duration,
}

impl RateLimitConfig {
    fn from_env() -> Self {
        Self {
            max_attempts: parse_env("RATE_LIMIT_MAX_ATTEMPTS", 10),
            window: Duration::from_secs(parse_env("RATE_LIMIT_WINDOW_SECONDS", 60)),
        }
    }
}

/// Retention for the `request_logs` observability table. Rows older than
/// `retention_days` are deleted by the hourly cleanup tick.
///
/// Env var: `REQUEST_LOG_RETENTION_DAYS` (default 7).
#[derive(Debug, Clone)]
pub struct RequestLogConfig {
    pub retention_days: i64,
}

impl RequestLogConfig {
    fn from_env() -> Self {
        Self {
            retention_days: parse_env("REQUEST_LOG_RETENTION_DAYS", 7),
        }
    }
}

/// Yggdrasil (Mojang-compatible auth) configuration.
///
/// Env vars: `YGGDRASIL_PUBLIC_URL` (default `/api/yggdrasil`),
/// `YGGDRASIL_KEY_PATH` (default `data/yggdrasil/keys/active.key.pem`),
/// `YGGDRASIL_TOKEN_TTL_SECONDS` (default 1296000 = 15d),
/// `YGGDRASIL_MAX_TOKENS_PER_USER` (default 10),
/// `YGGDRASIL_SKIN_DOMAINS` (comma-separated, default `.loontail.com,localhost`).
#[derive(Debug, Clone)]
pub struct YggdrasilConfig {
    pub public_url: String,
    pub key_path: String,
    pub token_ttl: Duration,
    pub max_tokens_per_user: i64,
    pub skin_domains: Vec<String>,
}

/// Skin/cape texture storage configuration.
///
/// Env var: `TEXTURES_STORAGE_ROOT` (default `data/textures`).
#[derive(Debug, Clone)]
pub struct TexturesConfig {
    pub storage_root: String,
}

/// Catalog media (client poster/background/titleImage/screenshots) storage
/// configuration.
///
/// Env var: `CATALOG_MEDIA_STORAGE_ROOT` (default `data/catalog-media`).
#[derive(Debug, Clone)]
pub struct CatalogConfig {
    pub storage_root: String,
}

/// Bundle-registry storage configuration.
///
/// Env vars: `BUNDLES_STORAGE_ROOT` (default `data/bundle-registry`),
/// `BUNDLES_PUBLIC_URL` (default `/bundle-registry`).
#[derive(Debug, Clone)]
pub struct BundlesConfig {
    pub storage_root: String,
    pub public_url: String,
}

/// Admin-panel session + bootstrap configuration.
///
/// Env vars: `ADMIN_SESSION_TTL_SECONDS` (default 604800 = 7d),
/// `ADMIN_COOKIE_NAME` (default `loontail_admin`),
/// `ADMIN_BOOTSTRAP_USERNAME` (default `admin`),
/// `ADMIN_BOOTSTRAP_PASSWORD` (optional — no seed admin created when unset).
#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub session_ttl: Duration,
    pub cookie_name: String,
    pub bootstrap_username: String,
    pub bootstrap_password: Option<String>,
}

impl Config {
    /// Build configuration from environment variables, applying sane defaults
    /// for everything except `DATABASE_URL`, which is required.
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url =
            env::var("DATABASE_URL").map_err(|_| anyhow::anyhow!("DATABASE_URL must be set"))?;

        // why: default CLOSED (empty), not `*`. An unset CORS_ALLOWED_ORIGINS
        // blocks all browser cross-origin callers rather than failing open; an
        // explicit `*` must be opted into and never combines with credentials.
        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_default()
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
            rate_limit: RateLimitConfig::from_env(),
            request_log: RequestLogConfig::from_env(),
            yggdrasil: YggdrasilConfig::from_env(),
            textures: TexturesConfig::from_env(),
            catalog: CatalogConfig::from_env(),
            bundles: BundlesConfig::from_env(),
            admin: AdminConfig::from_env(),
        })
    }
}

impl YggdrasilConfig {
    fn from_env() -> Self {
        let skin_domains = env::var("YGGDRASIL_SKIN_DOMAINS")
            .unwrap_or_else(|_| ".loontail.com,localhost".to_string())
            .split(',')
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect();
        Self {
            public_url: env::var("YGGDRASIL_PUBLIC_URL")
                .unwrap_or_else(|_| "/api/yggdrasil".to_string()),
            key_path: env::var("YGGDRASIL_KEY_PATH")
                .unwrap_or_else(|_| "data/yggdrasil/keys/active.key.pem".to_string()),
            token_ttl: Duration::from_secs(parse_env("YGGDRASIL_TOKEN_TTL_SECONDS", 1_296_000)),
            max_tokens_per_user: parse_env("YGGDRASIL_MAX_TOKENS_PER_USER", 10),
            skin_domains,
        }
    }
}

impl TexturesConfig {
    fn from_env() -> Self {
        Self {
            storage_root: env::var("TEXTURES_STORAGE_ROOT")
                .unwrap_or_else(|_| "data/textures".to_string()),
        }
    }
}

impl CatalogConfig {
    fn from_env() -> Self {
        Self {
            storage_root: env::var("CATALOG_MEDIA_STORAGE_ROOT")
                .unwrap_or_else(|_| "data/catalog-media".to_string()),
        }
    }
}

impl BundlesConfig {
    fn from_env() -> Self {
        Self {
            storage_root: env::var("BUNDLES_STORAGE_ROOT")
                .unwrap_or_else(|_| "data/bundle-registry".to_string()),
            public_url: env::var("BUNDLES_PUBLIC_URL")
                .unwrap_or_else(|_| "/bundle-registry".to_string()),
        }
    }
}

impl AdminConfig {
    fn from_env() -> Self {
        Self {
            session_ttl: Duration::from_secs(parse_env("ADMIN_SESSION_TTL_SECONDS", 604_800)),
            cookie_name: env::var("ADMIN_COOKIE_NAME")
                .unwrap_or_else(|_| "loontail_admin".to_string()),
            bootstrap_username: env::var("ADMIN_BOOTSTRAP_USERNAME")
                .unwrap_or_else(|_| "admin".to_string()),
            bootstrap_password: env::var("ADMIN_BOOTSTRAP_PASSWORD")
                .ok()
                .filter(|p| !p.is_empty()),
        }
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
