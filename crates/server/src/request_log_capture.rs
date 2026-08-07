//! Request-capture middleware: records served requests into `state.request_log`,
//! skipping probes and WebSocket upgrades (see [`is_excluded`]).

use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::header::{CONTENT_LENGTH, USER_AGENT};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;

use loontail_core::request_log::{resolve_principal, PrincipalSlot, RequestLog};
use loontail_core::AppState;

use crate::ip;

pub async fn capture(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
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
    let peer = ip::peer_from_extensions(request.extensions().get::<ConnectInfo<SocketAddr>>());
    let ip = ip::client_ip(&headers, peer, state.config.trusted_proxy).map(|addr| addr.to_string());
    let user_agent = header_string(&headers, &USER_AGENT);

    // why: the auth extractor is about to run the exact `sessions JOIN users` lookup we
    // need, so hand it a slot and read the answer afterwards instead of doubling the
    // hottest query in the system. Routes with no extractor (texture reads, the
    // Yggdrasil protocol) leave it empty and fall back below.
    let slot = PrincipalSlot::default();
    request.extensions_mut().insert(slot.clone());

    let started = Instant::now();
    let response = next.run(request).await;
    let latency_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;

    let principal = match slot.get() {
        Some(resolved) => resolved.clone(),
        None => resolve_principal(&state.pool, &headers, &state.config.admin.cookie_name).await,
    };

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
/// upgrade endpoints. Matched on the route TEMPLATE, so `/relay/{id}` is covered by
/// its prefix.
fn is_excluded(path: &str) -> bool {
    path == "/health" || path == "/metrics" || path == "/signaling" || path.starts_with("/relay")
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
    use super::{capture, is_excluded};

    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::{get, post};
    use axum::Router;
    use loontail_core::auth::{issue_session, AdminUser};
    use loontail_core::config::Config;
    use loontail_core::identity::{admin_create_user, AdminCreateUser};
    use loontail_core::request_log::PrincipalSlot;
    use loontail_core::AppState;
    use sqlx::PgPool;
    use tower::util::ServiceExt;

    fn test_state(pool: PgPool) -> AppState {
        // why: only a placeholder when the var is absent — clobbering a real DATABASE_URL
        // trips sqlx::test's "DATABASE_URL changed at runtime" assertion mid-run.
        if std::env::var_os("DATABASE_URL").is_none() {
            std::env::set_var("DATABASE_URL", "postgres://unused");
        }
        AppState::new(pool, Config::from_env().expect("config"))
    }

    /// A guarded handler that revokes its OWN session before returning. The extractor
    /// resolved the admin; a post-response re-query never could. So the logged
    /// attribution proves which one the middleware used.
    async fn revoke_self(
        axum::extract::State(state): axum::extract::State<AppState>,
        admin: AdminUser,
    ) -> &'static str {
        sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1")
            .bind(admin.id())
            .execute(&state.pool)
            .await
            .expect("revoke");
        "ok"
    }

    fn logging_app(state: AppState) -> Router {
        Router::new()
            .route("/api/guarded", get(revoke_self))
            .route("/api/open", get(|| async { "ok" }))
            .route("/api/mutate", post(|_admin: AdminUser| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state.clone(), capture))
            .with_state(state)
    }

    /// BE-06: the middleware must reuse the principal the auth extractor already
    /// resolved instead of issuing a second `sessions JOIN users` lookup. Asserted via a
    /// handler that revokes its own session — the shared slot still attributes the admin,
    /// whereas a post-response re-query would see the revoked session and log `anon`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticated_request_is_attributed_from_the_shared_slot(pool: PgPool) {
        let state = test_state(pool.clone());
        let admin = admin_create_user(
            &pool,
            AdminCreateUser {
                username: "reqlogadmin".into(),
                email: "reqlogadmin@example.com".into(),
                password: "pw".into(),
                minecraft_uuid: None,
                is_admin: true,
            },
        )
        .await
        .expect("admin");
        let token = issue_session(&pool, admin.id, std::time::Duration::from_secs(900))
            .await
            .expect("session")
            .token;

        let app = logging_app(state.clone());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/guarded")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let recent = state.request_log.recent(1);
        assert_eq!(recent.len(), 1, "one request captured");
        assert_eq!(
            recent[0].auth_kind, "admin",
            "attribution comes from the extractor's lookup, not a post-response re-query"
        );
        assert_eq!(recent[0].user_id, Some(admin.id));
    }

    /// A route with no auth extractor leaves the slot empty, so the fallback query still
    /// attributes it — as `anon` when no credential was presented.
    #[sqlx::test(migrations = "../../migrations")]
    async fn unauthenticated_route_falls_back_to_anon(pool: PgPool) {
        let state = test_state(pool.clone());
        let app = logging_app(state.clone());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/open")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let recent = state.request_log.recent(1);
        assert_eq!(recent[0].auth_kind, "anon");
        assert!(recent[0].user_id.is_none());
    }

    /// A rejected request records `anon` from the extractor's own error path — no second
    /// lookup for a credential that just failed.
    #[sqlx::test(migrations = "../../migrations")]
    async fn rejected_request_is_recorded_as_anon(pool: PgPool) {
        let state = test_state(pool.clone());
        let app = logging_app(state.clone());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/guarded")
                    .header("authorization", "Bearer not-a-real-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let recent = state.request_log.recent(1);
        assert_eq!(recent[0].auth_kind, "anon");
        assert!(recent[0].user_id.is_none());
    }

    /// A CSRF-rejected COOKIE mutation must still name the session it rode. The
    /// double-submit check runs before any lookup fails, so the extractor must leave the
    /// slot empty and let the middleware resolve the cookie — logging it as `anon` would
    /// erase the only record of whose session was used cross-site.
    #[sqlx::test(migrations = "../../migrations")]
    async fn csrf_rejected_cookie_mutation_is_attributed_to_the_cookie_owner(pool: PgPool) {
        let state = test_state(pool.clone());
        let admin = admin_create_user(
            &pool,
            AdminCreateUser {
                username: "csrfadmin".into(),
                email: "csrfadmin@example.com".into(),
                password: "pw".into(),
                minecraft_uuid: None,
                is_admin: true,
            },
        )
        .await
        .expect("admin");
        let token = issue_session(&pool, admin.id, std::time::Duration::from_secs(900))
            .await
            .expect("session")
            .token;

        let cookie_name = state.config.admin.cookie_name.clone();
        let app = logging_app(state.clone());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/mutate")
                    // No `x-csrf-token` header: exactly the cross-site form post.
                    .header("cookie", format!("{cookie_name}={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        let recent = state.request_log.recent(1);
        assert_eq!(
            recent[0].user_id,
            Some(admin.id),
            "the audit row must name the ridden session's owner, not NULL"
        );
        assert_eq!(recent[0].auth_kind, "admin");
    }

    #[test]
    fn slot_is_first_writer_wins() {
        use loontail_core::request_log::ResolvedPrincipal;

        let slot = PrincipalSlot::default();
        assert!(slot.get().is_none(), "empty until the extractor writes");
        slot.set(ResolvedPrincipal {
            user_id: None,
            auth_kind: "admin",
        });
        slot.set(ResolvedPrincipal::anon());
        assert_eq!(
            slot.get().expect("filled").auth_kind,
            "admin",
            "a later write must not overwrite the extractor's answer"
        );
    }

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
