use std::env;
use std::time::Duration;

/// Runtime configuration, loaded once from the environment at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub http_host: String,
    pub http_port: u16,
    pub database_url: String,
    pub cors_allowed_origins: Vec<String>,
    /// When `true`, the client IP is taken from the proxy-set `X-Forwarded-For` /
    /// `X-Real-IP` headers; when `false` (default) the transport peer is used and
    /// forwarding headers are never trusted. Set `true` only when the immediate peer
    /// is a proxy you control that overwrites these headers.
    pub trusted_proxy: bool,
    /// Optional shared bearer token authorizing `GET /metrics` for an ops scraper
    /// that can't carry an admin session. When unset, `/metrics` is reachable only
    /// by an authenticated admin. Env: `METRICS_TOKEN`.
    pub metrics_token: Option<String>,
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
    pub event_retention: EventRetentionConfig,
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

/// Retention for the join/invite lifecycle history and `user_events`. Terminal
/// join-request/invite rows, spent join tickets, and analytics events older than this
/// are deleted by the hourly cleanup tick. Kept longer than the request log because
/// these rows back the admin analytics views.
///
/// Env var: `EVENT_RETENTION_DAYS` (default 30).
#[derive(Debug, Clone)]
pub struct EventRetentionConfig {
    pub retention_days: i64,
}

impl EventRetentionConfig {
    fn from_env() -> Self {
        Self {
            retention_days: parse_env("EVENT_RETENTION_DAYS", 30),
        }
    }
}

/// Yggdrasil (Mojang-compatible auth) configuration.
///
/// Env vars: `YGGDRASIL_PUBLIC_URL` (default `/api/yggdrasil`),
/// `YGGDRASIL_KEY_PATH` (default `data/yggdrasil/keys/active.key.pem`),
/// `YGGDRASIL_TOKEN_TTL_SECONDS` (default 1296000 = 15d),
/// `YGGDRASIL_MAX_TOKENS_PER_USER` (default 10),
/// `YGGDRASIL_SKIN_DOMAINS` (comma-separated, default `.loontail.dev,localhost`).
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

        // why: default CLOSED (empty), not `*` — an unset value blocks cross-origin
        // callers rather than failing open; `*` must be opted into explicitly.
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
            // why: default CLOSED — forwarding headers are client-controllable, so
            // honour them only when told the immediate peer is a trusted proxy.
            trusted_proxy: parse_bool_env("TRUSTED_PROXY", false),
            metrics_token: env::var("METRICS_TOKEN").ok().filter(|t| !t.is_empty()),
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
            event_retention: EventRetentionConfig::from_env(),
            yggdrasil: YggdrasilConfig::from_env(),
            textures: TexturesConfig::from_env(),
            catalog: CatalogConfig::from_env(),
            bundles: BundlesConfig::from_env(),
            admin: AdminConfig::from_env(),
        })
    }
}

/// Hosts advertised to clients as allowed texture sources. A deployment overrides
/// this with its own `NETWORK_DOMAIN` (see `scripts/deploy-remote.sh`); the default
/// covers the project's own `*.loontail.dev` hosts plus local development.
const DEFAULT_SKIN_DOMAINS: &str = ".loontail.dev,localhost";

/// Split a comma-separated skin-domain list, trimming blanks so a trailing comma or
/// padded entry is harmless.
fn parse_skin_domains(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect()
}

impl YggdrasilConfig {
    fn from_env() -> Self {
        let skin_domains = parse_skin_domains(
            &env::var("YGGDRASIL_SKIN_DOMAINS")
                .unwrap_or_else(|_| DEFAULT_SKIN_DOMAINS.to_string()),
        );
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

/// Parse a boolean env var, accepting `true`/`1`/`yes`/`on` (case-insensitive) as
/// `true` and everything else (including unset) as `default`.
fn parse_bool_env(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

/// Split a configured public URL into its `scheme://authority` origin (when it
/// carries one) and its path portion; a path-only value yields `(None, path)`. The
/// textures crate uses the origin to absolutize asset URLs and the server uses the
/// path to mount the yggdrasil router, so this is the single source of truth.
pub fn parse_public_url(public_url: &str) -> (Option<String>, &str) {
    let Some(scheme_end) = public_url.find("://") else {
        return (None, public_url);
    };
    let after_scheme = scheme_end + 3;
    let rest = &public_url[after_scheme..];
    let authority_len = rest.find('/').unwrap_or(rest.len());
    if authority_len == 0 {
        // `scheme://` with no authority — treat the remainder as a path.
        return (None, &public_url[after_scheme..]);
    }
    let origin = public_url[..after_scheme + authority_len].to_string();
    (Some(origin), &public_url[after_scheme + authority_len..])
}

#[cfg(test)]
mod public_url_tests {
    use super::parse_public_url;

    #[test]
    fn full_url_splits_into_origin_and_path() {
        assert_eq!(
            parse_public_url("https://auth.loontail.com/api/yggdrasil"),
            (
                Some("https://auth.loontail.com".to_string()),
                "/api/yggdrasil"
            )
        );
        assert_eq!(
            parse_public_url("http://localhost:8080/api/yggdrasil"),
            (Some("http://localhost:8080".to_string()), "/api/yggdrasil")
        );
    }

    #[test]
    fn bare_origin_has_empty_path() {
        assert_eq!(
            parse_public_url("https://auth.loontail.com"),
            (Some("https://auth.loontail.com".to_string()), "")
        );
    }

    #[test]
    fn path_only_has_no_origin() {
        assert_eq!(parse_public_url("/api/yggdrasil"), (None, "/api/yggdrasil"));
        assert_eq!(parse_public_url(""), (None, ""));
    }
}

#[cfg(test)]
mod skin_domain_tests {
    use super::{parse_skin_domains, DEFAULT_SKIN_DOMAINS};

    /// CON-02: the default whitelist must cover the domains this project actually
    /// deploys on (`*.loontail.dev`), or clients refuse every texture.
    #[test]
    fn default_skin_domains_cover_the_real_deployment() {
        assert_eq!(
            parse_skin_domains(DEFAULT_SKIN_DOMAINS),
            vec![".loontail.dev".to_string(), "localhost".to_string()]
        );
    }

    #[test]
    fn blank_and_padded_entries_are_dropped() {
        assert_eq!(
            parse_skin_domains(" cms.loontail.dev , .loontail.dev ,, "),
            vec!["cms.loontail.dev".to_string(), ".loontail.dev".to_string()]
        );
    }
}
