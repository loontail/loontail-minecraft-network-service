use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::metrics::Metrics;
use crate::relay::RelayHub;
use crate::signaling::SignalingHub;

/// Shared application state, cloned into every handler. All non-trivial fields
/// are behind `Arc` so cloning stays cheap.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub signaling: Arc<SignalingHub>,
    pub relay: Arc<RelayHub>,
    pub metrics: Arc<Metrics>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        AppState {
            pool,
            config: Arc::new(config),
            signaling: Arc::new(SignalingHub::new()),
            relay: Arc::new(RelayHub::new()),
            metrics: Arc::new(Metrics::new()),
        }
    }
}
