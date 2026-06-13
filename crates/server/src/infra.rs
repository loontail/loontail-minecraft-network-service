use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use loontail_core::AppState;

/// `GET /health` — liveness + database connectivity. Returns 503 if the DB
/// cannot be reached, so orchestrators can restart the container.
pub async fn health(State(state): State<AppState>) -> Response {
    let database_up = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();

    let (status, label) = if database_up {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "degraded")
    };

    (
        status,
        Json(json!({
            "status": label,
            "database": if database_up { "up" } else { "down" },
        })),
    )
        .into_response()
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
        state.realtime.relay.active_pairings()
    ));
    body.push_str(&format!(
        "# TYPE loontail_signaling_active gauge\nloontail_signaling_active {}\n",
        state.realtime.signaling.connected_users()
    ));

    ([("content-type", "text/plain; version=0.0.4")], body)
}
