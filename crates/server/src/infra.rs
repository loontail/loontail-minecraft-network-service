use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use loontail_core::auth::{bearer_token_from_headers, AdminUser};
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
///
/// Gated (SEC-2): accepts an authenticated admin session OR the configured
/// `METRICS_TOKEN` as a Bearer (for header-only scrapers); 401/403 otherwise.
pub async fn metrics_handler(state: State<AppState>, headers: HeaderMap) -> Response {
    if let Err(rejection) = authorize_metrics(&state, &headers).await {
        return rejection;
    }
    let State(state) = state;
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

    ([("content-type", "text/plain; version=0.0.4")], body).into_response()
}

/// Authorize a `/metrics` request: a valid admin session, or a Bearer matching the
/// configured `METRICS_TOKEN` (constant-time). On failure returns the rejection
/// `Response` to short-circuit with.
async fn authorize_metrics(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    if let (Some(expected), Some(presented)) = (
        &state.config.metrics_token,
        bearer_token_from_headers(headers),
    ) {
        if loontail_core::auth::constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
            return Ok(());
        }
    }

    // Fall back to an admin session (cookie or Bearer) via the standard extractor.
    let mut req = axum::http::Request::new(axum::body::Body::empty());
    *req.headers_mut() = headers.clone();
    let (mut parts, _) = req.into_parts();
    match <AdminUser as axum::extract::FromRequestParts<AppState>>::from_request_parts(
        &mut parts, state,
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(err) => Err(err.into_response()),
    }
}
