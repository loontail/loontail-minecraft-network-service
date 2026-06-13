//! Integration tests for the admin domain: bootstrap admin seeding, login
//! (admin-only), Yggdrasil-user creation proving end-to-end credential binding,
//! block/reset/revoke, API-token create+verify, analytics overview from seeded
//! presence, and CSRF rejection. Each test runs against an isolated Postgres via
//! `#[sqlx::test]` and drives a `Router<AppState>` through `tower::oneshot`.

use std::time::Duration;

use axum::body::Body;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use loontail_admin::{ensure_bootstrap_admin, verify_api_token};
use loontail_core::auth::CSRF_COOKIE_NAME;
use loontail_core::config::Config;
use loontail_core::identity::authenticate_password;
use loontail_core::AppState;

const COOKIE_NAME: &str = "loontail_admin";

fn test_state(pool: PgPool) -> AppState {
    std::env::set_var("DATABASE_URL", "postgres://unused");
    let mut config = Config::from_env().unwrap();
    config.admin.cookie_name = COOKIE_NAME.to_string();
    config.admin.bootstrap_username = "rootadmin".to_string();
    config.admin.bootstrap_password = Some("rootpass123".to_string());
    config.admin.session_ttl = Duration::from_secs(900);
    // Generous heartbeat window so seeded presence rows count as live.
    config.heartbeat_timeout = Duration::from_secs(3600);
    AppState::new(pool, config)
}

/// Mount the admin router exactly as the server integration does: the routed
/// `Router<AppState>` is given its state, then `nest_service`-mounted under
/// `/admin`. `nest_service` (vs `nest`) is what lets the SPA shell answer the
/// bare `/admin/` (trailing-slash) request — a plain `nest` 404s that exact path.
fn app(state: AppState) -> Router {
    Router::new().nest_service("/admin", loontail_admin::routes().with_state(state))
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Extract a named cookie value from all Set-Cookie headers on a response.
fn cookie_from(resp: &axum::response::Response, name: &str) -> Option<String> {
    resp.headers().get_all(SET_COOKIE).iter().find_map(|hv| {
        let s = hv.to_str().ok()?;
        let first = s.split(';').next()?;
        let (k, v) = first.split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

/// A logged-in admin session: the session cookie and CSRF token to drive
/// authenticated, CSRF-protected requests.
#[derive(Debug)]
struct Session {
    session_cookie: String,
    csrf: String,
}

impl Session {
    fn cookie_header(&self) -> String {
        format!(
            "{COOKIE_NAME}={}; {CSRF_COOKIE_NAME}={}",
            self.session_cookie, self.csrf
        )
    }
}

async fn login(app: &Router, username: &str, password: &str) -> Result<Session, StatusCode> {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "username": username, "password": password }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    if resp.status() != StatusCode::OK {
        return Err(resp.status());
    }
    let session_cookie = cookie_from(&resp, COOKIE_NAME).expect("session cookie set");
    let csrf = cookie_from(&resp, CSRF_COOKIE_NAME).expect("csrf cookie set");
    Ok(Session {
        session_cookie,
        csrf,
    })
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_seeds_admin_and_is_idempotent(pool: PgPool) {
    let state = test_state(pool.clone());
    let created = ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    assert!(created, "first run creates the bootstrap admin");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE is_admin = true")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let again = ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    assert!(!again, "second run is a no-op once an admin exists");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE is_admin = true")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_rejects_non_admin_accepts_admin_and_me_works(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    let app = app(state);

    // A non-admin credential exists but must be refused at login (403).
    loontail_core::identity::admin_create_user(
        &pool,
        loontail_core::identity::AdminCreateUser {
            username: "peon".into(),
            email: "peon@example.com".into(),
            password: "peonpass".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();

    let err = login(&app, "peon", "peonpass").await.unwrap_err();
    assert_eq!(err, StatusCode::FORBIDDEN);

    // Wrong password for the admin is a 403 too.
    assert_eq!(
        login(&app, "rootadmin", "wrong").await.unwrap_err(),
        StatusCode::FORBIDDEN
    );

    // The bootstrap admin logs in and the session works on /auth/me.
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/auth/me")
                .header(COOKIE, format!("{COOKIE_NAME}={}", session.session_cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let me = body_json(resp).await;
    assert_eq!(me["username"], "rootadmin");
    assert_eq!(me["isAdmin"], true);

    // No cookie ⇒ unauthorized.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_yggdrasil_user_then_password_auth_succeeds(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    let app = app(state);
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/users")
                .header("content-type", "application/json")
                .header(COOKIE, session.cookie_header())
                .header("x-csrf-token", &session.csrf)
                .body(Body::from(
                    json!({
                        "username": "alice",
                        "email": "alice@example.com",
                        "password": "alicepass"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    assert_eq!(created["username"], "alice");
    assert_eq!(created["origin"], "admin");
    assert_eq!(created["confirmed"], true);
    assert!(created["profileUuid"].as_str().unwrap().len() == 32);

    // End-to-end binding: the user the admin just created can authenticate.
    let user = authenticate_password(&pool, "alice", "alicepass")
        .await
        .unwrap();
    assert_eq!(user.username, "alice");
    // Authenticating by email works too.
    assert!(
        authenticate_password(&pool, "alice@example.com", "alicepass")
            .await
            .is_ok()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_user_without_csrf_is_rejected(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    let app = app(state);
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();

    // Session cookie present (so the AdminUser extractor passes) but no
    // x-csrf-token header ⇒ the mutation is forbidden.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/users")
                .header("content-type", "application/json")
                .header(COOKIE, session.cookie_header())
                .body(Body::from(
                    json!({
                        "username": "bob",
                        "email": "bob@example.com",
                        "password": "bobpass"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE normalized_username = 'bob'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0, "no user created when CSRF check fails");
}

#[sqlx::test(migrations = "../../migrations")]
async fn block_reset_and_revoke_tokens(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    let app = app(state);
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();

    // Create a target user and give it a yggdrasil token + a network session.
    let target = loontail_core::identity::admin_create_user(
        &pool,
        loontail_core::identity::AdminCreateUser {
            username: "carol".into(),
            email: "carol@example.com".into(),
            password: "carolpass".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    let ygg = loontail_core::auth::yggdrasil::issue_yggdrasil_tokens(
        &pool,
        target.id,
        None,
        Duration::from_secs(900),
        10,
    )
    .await
    .unwrap();
    assert!(
        loontail_core::auth::yggdrasil::validate_yggdrasil(&pool, &ygg.access_token, None)
            .await
            .is_ok()
    );

    let auth = |path: String, method: &'static str| {
        let app = app.clone();
        let cookie = session.cookie_header();
        let csrf = session.csrf.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(COOKIE, cookie)
                    .header("x-csrf-token", csrf)
                    .header("content-type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // Block: credential login is refused afterwards.
    let resp = auth(format!("/admin/users/{}/block", target.id), "POST").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(authenticate_password(&pool, "carol", "carolpass")
        .await
        .is_err());

    // Unblock restores login.
    let resp = auth(format!("/admin/users/{}/unblock", target.id), "POST").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(authenticate_password(&pool, "carol", "carolpass")
        .await
        .is_ok());

    // Reset password: old password fails, new one (set directly below for the
    // assertion) works; existing yggdrasil tokens are invalidated.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/users/{}/reset-password", target.id))
                .header(COOKIE, session.cookie_header())
                .header("x-csrf-token", &session.csrf)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "password": "newcarolpass" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(authenticate_password(&pool, "carol", "carolpass")
        .await
        .is_err());
    assert!(authenticate_password(&pool, "carol", "newcarolpass")
        .await
        .is_ok());
    assert!(
        loontail_core::auth::yggdrasil::validate_yggdrasil(&pool, &ygg.access_token, None)
            .await
            .is_err(),
        "reset invalidates yggdrasil tokens"
    );

    // revoke-tokens on a fresh token also clears it.
    let ygg2 = loontail_core::auth::yggdrasil::issue_yggdrasil_tokens(
        &pool,
        target.id,
        None,
        Duration::from_secs(900),
        10,
    )
    .await
    .unwrap();
    let resp = auth(format!("/admin/users/{}/revoke-tokens", target.id), "POST").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        loontail_core::auth::yggdrasil::validate_yggdrasil(&pool, &ygg2.access_token, None)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn api_token_create_and_verify(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    let app = app(state.clone());
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/api-tokens")
                .header(COOKIE, session.cookie_header())
                .header("x-csrf-token", &session.csrf)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "name": "launcher", "scopes": ["catalog:read"] }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    let raw = created["token"].as_str().unwrap().to_string();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(raw.len(), 64);

    // The raw token verifies against the stored hash.
    let resolved = verify_api_token(&state, &raw).await.unwrap();
    assert_eq!(resolved.to_string(), id);
    // A bogus token does not.
    assert!(verify_api_token(&state, "nope").await.is_err());

    // It shows up in the list (metadata only, no raw value).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/api-tokens")
                .header(COOKIE, format!("{COOKIE_NAME}={}", session.session_cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "launcher");
    assert!(list[0].get("token").is_none());

    // Delete it.
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/admin/api-tokens/{id}"))
                .header(COOKIE, session.cookie_header())
                .header("x-csrf-token", &session.csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(verify_api_token(&state, &raw).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn analytics_overview_counts_seeded_rows(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    // Seed three users with presence: one online, one joinable (playing), one
    // inWorld (playing), and a fourth whose heartbeat is stale (offline).
    let mut ids = Vec::new();
    for (name, status, stale) in [
        ("u_online", "online", false),
        ("u_joinable", "joinable", false),
        ("u_inworld", "inWorld", false),
        ("u_stale", "online", true),
    ] {
        let u = loontail_core::identity::admin_create_user(
            &pool,
            loontail_core::identity::AdminCreateUser {
                username: name.into(),
                email: format!("{name}@example.com"),
                password: "pw".into(),
                minecraft_uuid: None,
                is_admin: false,
            },
        )
        .await
        .unwrap();
        let hb = if stale {
            "now() - interval '2 hours'"
        } else {
            "now()"
        };
        sqlx::query(&format!(
            "INSERT INTO presence (user_id, status, last_heartbeat_at) VALUES ($1, $2, {hb})"
        ))
        .bind(u.id)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
        ids.push(u.id);
    }

    // An open world session + an active relay session for the host.
    let host = ids[2];
    let world: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO world_sessions (host_user_id, status) VALUES ($1, 'open') RETURNING id",
    )
    .bind(host)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO relay_sessions (world_session_id, host_user_id, status) VALUES ($1, $2, 'active')",
    )
    .bind(world)
    .bind(host)
    .execute(&pool)
    .await
    .unwrap();

    let app = app(state);
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/analytics/overview")
                .header(COOKIE, format!("{COOKIE_NAME}={}", session.session_cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let overview = body_json(resp).await;
    assert_eq!(overview["playingNow"], 2, "joinable + inWorld, live");
    assert_eq!(
        overview["onlineInNetwork"], 3,
        "online + joinable + inWorld, live (stale excluded)"
    );
    assert_eq!(overview["openWorlds"], 1);
    assert_eq!(overview["activeRelays"], 1);
    // 5 = 1 bootstrap admin + 4 seeded.
    assert_eq!(overview["totalUsers"], 5);
}

#[sqlx::test(migrations = "../../migrations")]
async fn analytics_timeseries_reads_user_events(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    // Record a couple of bootstrap events via the helper.
    loontail_admin::analytics::record_event(&state, "bootstrap", None, None)
        .await
        .unwrap();
    loontail_admin::analytics::record_event(&state, "bootstrap", None, None)
        .await
        .unwrap();

    let app = app(state);
    let session = login(&app, "rootadmin", "rootpass123").await.unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/analytics/timeseries?metric=bootstraps&window=7d")
                .header(COOKIE, format!("{COOKIE_NAME}={}", session.session_cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ts = body_json(resp).await;
    assert_eq!(ts["metric"], "bootstraps");
    let series = ts["series"].as_array().unwrap();
    let total: i64 = series.iter().map(|p| p["count"].as_i64().unwrap()).sum();
    assert_eq!(total, 2);
}

/// Fetch the embedded SPA shell and extract the hashed JS bundle path it
/// references (e.g. `/admin/assets/index-XXXX.js`). The placeholder shell that
/// `build.rs` writes when `admin-ui/dist` is absent has no such reference, so
/// this also asserts the real built SPA is embedded.
async fn spa_asset_path(app: &Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    let marker = "/admin/assets/";
    let start = html
        .find(marker)
        .expect("built SPA shell references a hashed /admin/assets/*.js bundle");
    let rest = &html[start..];
    let end = rest
        .find(".js")
        .expect("asset reference is a .js bundle")
        + ".js".len();
    rest[..end].to_string()
}

#[sqlx::test(migrations = "../../migrations")]
async fn spa_serves_shell_assets_and_client_routes(pool: PgPool) {
    let state = test_state(pool);
    let app = app(state);

    // The shell answers the bare `/admin/` (trailing slash).
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(
        String::from_utf8_lossy(&bytes).contains("<div id=\"root\""),
        "serves the SPA shell at /admin/"
    );

    // A real embedded asset is served as JavaScript (not the SPA shell). The
    // hashed bundle name is discovered from the embedded index.html so the test
    // tracks the current build instead of a stale hardcoded hash.
    let asset_path = spa_asset_path(&app).await;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&asset_path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("javascript"),
        "asset served as JavaScript, got {content_type}"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("<div id=\"root\""),
        "asset response is the real bundle, not the SPA shell"
    );

    // An unknown client route falls back to the SPA shell (HTML), not a 404.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("<div id=\"root\""));

    // A client route that a REST endpoint also claims (`GET /admin/users`) still
    // serves the SPA shell for a browser page navigation (Accept: text/html), so
    // the route deep links and survives a refresh.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/users")
                .header("accept", "text/html,application/xhtml+xml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(
        String::from_utf8_lossy(&bytes).contains("<div id=\"root\""),
        "browser navigation to a REST-shadowed path serves the SPA shell"
    );

    // The same path requested as JSON (the SPA's own fetch) is not hijacked by the
    // shell: it reaches the REST handler, which rejects the unauthenticated call.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/users")
                .header("accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "JSON fetch reaches the REST handler, not the SPA shell"
    );
}
