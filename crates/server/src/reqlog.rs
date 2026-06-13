//! Request-capture middleware.
//!
//! Wraps the root router as a `from_fn_with_state` layer. For every served
//! request that is not on the noise allowlist it records the method, matched-route
//! template, status, latency, best-effort principal, client IP, user agent, and
//! response size into `state.request_log`. WebSocket upgrades (`/signaling`,
//! `/relay/*`) and the `/health`/`/metrics` probes are skipped so the log reflects
//! real API traffic, not long-lived sockets or monitoring polls.

use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::header::{CONTENT_LENGTH, USER_AGENT};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;

use loontail_core::request_log::{resolve_principal, RequestLog};
use loontail_core::AppState;

/// Records each served request into the observability sink. Skips the noise
/// allowlist; resolves the principal best-effort before running the handler so a
/// single cheap query (only when a token is present) attributes the row.
pub async fn capture(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // Matched-route TEMPLATE (e.g. `/admin/users/{id}`) so the log groups by route,
    // not by every distinct id. Falls back to the raw path when no route matched.
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());

    if is_excluded(&path) {
        return next.run(request).await;
    }

    let method = request.method().as_str().to_string();
    let headers = request.headers().clone();
    let ip = resolve_ip(
        request.extensions().get::<ConnectInfo<SocketAddr>>(),
        &headers,
    );
    let user_agent = header_string(&headers, &USER_AGENT);

    let principal = resolve_principal(&state.pool, &headers, &state.config.admin.cookie_name).await;

    let started = Instant::now();
    let response = next.run(request).await;
    let latency_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;

    let status = response.status().as_u16() as i16;
    let bytes_out = content_length(response.headers());

    state.request_log.record(RequestLog {
        ts: Utc::now(),
        method,
        path,
        status,
        latency_ms,
        user_id: principal.user_id,
        auth_kind: principal.auth_kind.to_string(),
        ip,
        user_agent,
        bytes_out,
    });

    response
}

/// Paths excluded from request logging: liveness/metrics probes and the WebSocket
/// upgrade endpoints (matched on the route TEMPLATE, so `/relay/{id}` style routes
/// are covered by their prefix).
fn is_excluded(path: &str) -> bool {
    path == "/health" || path == "/metrics" || path == "/signaling" || path.starts_with("/relay")
}

/// Resolve the client IP: the transport peer from `ConnectInfo`, else the first
/// hop of `X-Forwarded-For` (only reachable behind a trusted proxy that strips the
/// socket and overwrites the header).
fn resolve_ip(
    connect_info: Option<&ConnectInfo<SocketAddr>>,
    headers: &HeaderMap,
) -> Option<String> {
    if let Some(ConnectInfo(addr)) = connect_info {
        return Some(addr.ip().to_string());
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .map(|first| first.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn content_length(headers: &HeaderMap) -> Option<i64> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
}

fn header_string(headers: &HeaderMap, name: &axum::http::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::is_excluded;

    #[test]
    fn excludes_probes_and_ws_upgrades() {
        assert!(is_excluded("/health"));
        assert!(is_excluded("/metrics"));
        assert!(is_excluded("/signaling"));
        assert!(is_excluded("/relay/abc"));
        assert!(is_excluded("/relay"));
    }

    #[test]
    fn records_api_admin_and_static_paths() {
        assert!(!is_excluded("/admin/users/{id}"));
        assert!(!is_excluded("/api/auth/login"));
        assert!(!is_excluded("/textures/skin.png"));
        assert!(!is_excluded("/bundle-registry/manifest"));
    }
}
