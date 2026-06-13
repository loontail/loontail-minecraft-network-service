use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight in-process counters. Kept deliberately small for the MVP;
/// the structure is ready to be swapped for a Prometheus registry later.
#[derive(Debug, Default)]
pub struct Metrics {
    pub bootstraps: AtomicU64,
    pub heartbeats: AtomicU64,
    pub friend_requests_created: AtomicU64,
    pub join_tickets_issued: AtomicU64,
    pub relay_sessions_opened: AtomicU64,
    pub relay_bytes_forwarded: AtomicU64,
    pub signaling_connections: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Metrics::default()
    }

    pub fn incr(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(counter: &AtomicU64, value: u64) {
        counter.fetch_add(value, Ordering::Relaxed);
    }
}
