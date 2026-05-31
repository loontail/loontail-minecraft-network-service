use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::AppState;

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

/// `GET /metrics` — Prometheus-style text exposition.
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let metrics = &state.metrics;
    let mut body = String::new();
    let lines = [
        ("loontail_bootstraps_total", &metrics.bootstraps),
        ("loontail_heartbeats_total", &metrics.heartbeats),
        (
            "loontail_friend_requests_created_total",
            &metrics.friend_requests_created,
        ),
        (
            "loontail_join_tickets_issued_total",
            &metrics.join_tickets_issued,
        ),
        (
            "loontail_relay_sessions_opened_total",
            &metrics.relay_sessions_opened,
        ),
        (
            "loontail_relay_bytes_forwarded_total",
            &metrics.relay_bytes_forwarded,
        ),
        (
            "loontail_signaling_connections_total",
            &metrics.signaling_connections,
        ),
    ];

    for (name, counter) in lines {
        let value = counter.load(Ordering::Relaxed);
        body.push_str(&format!("# TYPE {name} counter\n{name} {value}\n"));
    }

    body.push_str(&format!(
        "# TYPE loontail_relay_active gauge\nloontail_relay_active {}\n",
        state.relay.active_pairings()
    ));
    body.push_str(&format!(
        "# TYPE loontail_signaling_active gauge\nloontail_signaling_active {}\n",
        state.signaling.connected_users()
    ));

    ([("content-type", "text/plain; version=0.0.4")], body)
}
