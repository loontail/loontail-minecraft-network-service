//! Shared kernel for the Loontail launcher API: state, config, db, error,
//! metrics, models, auth, and the in-memory realtime structures. Domain crates
//! depend only on this crate and return `axum::Router<AppState>`.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod identity;
pub mod metrics;
pub mod models;
pub mod realtime;
pub mod request_log;
pub mod state;

pub use auth::{
    bearer_token_from_headers, cleanup_expired_sessions, generate_token, hash_token, issue_session,
    revoke_all_sessions_for_user, revoke_session, session_token_from_headers, user_from_token,
    AdminUser, AuthUser, IssuedSession, YggdrasilUser,
};
pub use config::{
    AdminConfig, BundlesConfig, CatalogConfig, Config, RateLimitConfig, RequestLogConfig,
    TexturesConfig, YggdrasilConfig,
};
pub use error::{AppError, AppResult};
pub use identity::{hash_password, verify_password};
pub use metrics::Metrics;
pub use realtime::{PendingPeer, Realtime, RelayHub, ServerEvent, SignalingHub};
pub use request_log::{
    delete_request_logs_older_than, resolve_principal, RequestLog, RequestLogSink,
    ResolvedPrincipal,
};
pub use state::AppState;
