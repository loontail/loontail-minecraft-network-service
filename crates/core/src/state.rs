use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::metrics::Metrics;
use crate::realtime::Realtime;
use crate::request_log::RequestLogSink;

/// Shared application state, cloned into every handler. All non-trivial fields
/// are behind `Arc` so cloning stays cheap.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub metrics: Arc<Metrics>,
    pub realtime: Arc<Realtime>,
    pub request_log: Arc<RequestLogSink>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let request_log = Arc::new(RequestLogSink::new(pool.clone()));
        AppState {
            pool,
            config: Arc::new(config),
            metrics: Arc::new(Metrics::new()),
            realtime: Arc::new(Realtime::new()),
            request_log,
        }
    }
}
