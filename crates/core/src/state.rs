use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::metrics::Metrics;
use crate::realtime::Realtime;

/// Shared application state, cloned into every handler. All non-trivial fields
/// are behind `Arc` so cloning stays cheap.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub metrics: Arc<Metrics>,
    pub realtime: Arc<Realtime>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        AppState {
            pool,
            config: Arc::new(config),
            metrics: Arc::new(Metrics::new()),
            realtime: Arc::new(Realtime::new()),
        }
    }
}
