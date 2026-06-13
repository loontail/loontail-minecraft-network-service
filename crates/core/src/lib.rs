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
pub mod state;

pub use auth::{
    bearer_token_from_headers, generate_token, hash_token, user_from_token, AdminUser, AuthUser,
    YggdrasilUser,
};
pub use config::{AdminConfig, BundlesConfig, Config, TexturesConfig, YggdrasilConfig};
pub use error::{AppError, AppResult};
pub use identity::{hash_password, verify_password};
pub use metrics::Metrics;
pub use realtime::{PendingPeer, Realtime, RelayHub, ServerEvent, SignalingHub};
pub use state::AppState;
