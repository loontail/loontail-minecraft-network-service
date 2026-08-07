//! Integration tests for the launcher/agent account surface (`/api/auth/*`).
//! These are the unified-session endpoints the launcher and in-game agent use:
//! register/login mint BOTH an opaque API session token (sent as a Bearer) and a
//! Yggdrasil access/client pair for the in-game handshake. Each test runs against
//! an isolated Postgres via `#[sqlx::test]` and drives `account_routes()` through
//! `tower::ServiceExt::oneshot` — no real socket is opened. Production code is not
//! touched; the only seam reached directly is `loontail_core::identity::block`,
//! to put an account into the blocked state the HTTP surface cannot reach.

use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use loontail_core::config::Config;
use loontail_core::AppState;
use loontail_yggdrasil::account_routes;

/// Build the account router with state wired to the injected test pool. The pool
/// comes from `#[sqlx::test]`, so the `DATABASE_URL` placeholder `Config::from_env`
/// requires is never actually dialed. The `/api/auth/*` handlers never touch the
/// Yggdrasil signing key, so `init_crypto` is intentionally NOT called.
fn app(pool: PgPool) -> Router {
    // why: only a placeholder when the var is absent — clobbering a real DATABASE_URL
    // trips sqlx::test's "DATABASE_URL changed at runtime" assertion mid-run.
    if std::env::var_os("DATABASE_URL").is_none() {
        std::env::set_var("DATABASE_URL", "postgres://unused");
    }
    let config = Config::from_env().unwrap();
    let state = AppState::new(pool, config);
    account_routes().with_state(state)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// POST a JSON body to `uri`, optionally with a Bearer session token.
async fn post(
    app: &Router,
    uri: &str,
    token: Option<&str>,
    body: Value,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

/// GET `uri`, optionally with a Bearer session token.
async fn get(app: &Router, uri: &str, token: Option<&str>) -> axum::response::Response {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Register `username`/`email` with a known-good password and assert the full
/// auth response shape, returning the parsed JSON for follow-up assertions.
async fn register_ok(app: &Router, username: &str, email: &str) -> Value {
    let resp = post(
        app,
        "/register",
        None,
        json!({ "username": username, "email": email, "password": "correct horse" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "register should succeed");
    body_json(resp).await
}

/// The session token a register/login response hands the launcher.
fn session_token(auth: &Value) -> &str {
    auth["session"]["token"].as_str().expect("session token")
}

#[sqlx::test(migrations = "../../migrations")]
async fn register_returns_session_minecraft_pair_and_nonadmin_profile(pool: PgPool) {
    let app = app(pool);

    let auth = register_ok(&app, "newbie", "newbie@example.com").await;

    // A session token AND its expiry are present.
    assert!(
        session_token(&auth).len() >= 32,
        "an opaque session token is returned"
    );
    assert!(
        auth["session"]["expiresAt"].as_str().is_some(),
        "the session carries an expiry"
    );

    // A Yggdrasil access/client pair is minted alongside the session.
    assert!(auth["minecraft"]["accessToken"].as_str().is_some());
    assert!(auth["minecraft"]["clientToken"].as_str().is_some());

    // The profile is a fresh, non-admin, confirmed account with a profile UUID.
    let profile = &auth["profile"];
    assert_eq!(profile["username"], "newbie");
    assert_eq!(profile["email"], "newbie@example.com");
    assert_eq!(profile["isAdmin"], false);
    assert!(profile["id"].as_str().is_some());
    assert_eq!(
        profile["uuid"].as_str().map(str::len),
        Some(32),
        "a random undashed profile uuid is assigned"
    );

    // The session token actually authorizes: /me echoes the same identity.
    let me = body_json(get(&app, "/me", Some(session_token(&auth))).await).await;
    assert_eq!(me["id"], profile["id"]);
    assert_eq!(me["username"], "newbie");
    assert_eq!(me["email"], "newbie@example.com");
    assert_eq!(me["uuid"], profile["uuid"]);
    assert_eq!(me["isAdmin"], false);
}

#[sqlx::test(migrations = "../../migrations")]
async fn register_rejects_duplicate_username(pool: PgPool) {
    let app = app(pool);
    register_ok(&app, "dupe", "first@example.com").await;

    // Same username, different email → 409 Conflict (username uniqueness).
    let resp = post(
        &app,
        "/register",
        None,
        json!({ "username": "dupe", "email": "second@example.com", "password": "another pw" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn register_rejects_duplicate_email_case_insensitively(pool: PgPool) {
    let app = app(pool);
    register_ok(&app, "caseone", "Person@Example.com").await;

    // A different username but an email differing only in CASE → 409 (the email
    // unique index is case-insensitive).
    let resp = post(
        &app,
        "/register",
        None,
        json!({ "username": "casetwo", "email": "person@example.COM", "password": "another pw" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn register_rejects_missing_or_empty_password(pool: PgPool) {
    let app = app(pool);

    // An empty password is a 400.
    let resp = post(
        &app,
        "/register",
        None,
        json!({ "username": "nopass", "email": "nopass@example.com", "password": "" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // A missing password field fails deserialization → 4xx (no account created).
    let resp = post(
        &app,
        "/register",
        None,
        json!({ "username": "nopass2", "email": "nopass2@example.com" }),
    )
    .await;
    assert!(
        resp.status().is_client_error(),
        "a missing password is rejected, got {}",
        resp.status()
    );

    // Neither attempt created a usable account: login with no password is 403.
    let resp = post(
        &app,
        "/login",
        None,
        json!({ "username": "nopass", "password": "" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_succeeds_by_username_and_email_and_authorizes(pool: PgPool) {
    let app = app(pool);
    let registered = register_ok(&app, "loginuser", "login@example.com").await;

    // Login by username.
    let by_name = body_json(
        post(
            &app,
            "/login",
            None,
            json!({ "username": "loginuser", "password": "correct horse" }),
        )
        .await,
    )
    .await;
    assert_eq!(by_name["profile"]["id"], registered["profile"]["id"]);

    // Login by email (the `username` field accepts an email).
    let by_email = body_json(
        post(
            &app,
            "/login",
            None,
            json!({ "username": "login@example.com", "password": "correct horse" }),
        )
        .await,
    )
    .await;
    assert_eq!(by_email["profile"]["id"], registered["profile"]["id"]);

    // The freshly issued session token authorizes /me.
    let me = body_json(get(&app, "/me", Some(session_token(&by_name))).await).await;
    assert_eq!(me["username"], "loginuser");
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_wrong_password_is_forbidden(pool: PgPool) {
    let app = app(pool);
    register_ok(&app, "pwuser", "pw@example.com").await;

    let resp = post(
        &app,
        "/login",
        None,
        json!({ "username": "pwuser", "password": "wrong password" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_unknown_user_is_forbidden(pool: PgPool) {
    let app = app(pool);

    let resp = post(
        &app,
        "/login",
        None,
        json!({ "username": "ghost", "password": "anything" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_on_blocked_account_is_forbidden(pool: PgPool) {
    let app = app(pool.clone());
    let registered = register_ok(&app, "blockme", "block@example.com").await;
    let user_id: uuid::Uuid = registered["profile"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Blocking is not reachable over the account surface; reach it via core.
    loontail_core::identity::block(&pool, user_id)
        .await
        .expect("block account");

    let resp = post(
        &app,
        "/login",
        None,
        json!({ "username": "blockme", "password": "correct horse" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn me_requires_a_bearer_token(pool: PgPool) {
    let app = app(pool);
    let auth = register_ok(&app, "meuser", "me@example.com").await;

    // With the session bearer, /me returns the account and matches register.
    let resp = get(&app, "/me", Some(session_token(&auth))).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let me = body_json(resp).await;
    assert_eq!(me["id"], auth["profile"]["id"]);
    assert_eq!(me["username"], "meuser");

    // Without a bearer, /me is 401.
    let resp = get(&app, "/me", None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // A bogus bearer is also 401.
    let resp = get(&app, "/me", Some("not-a-real-token")).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../../migrations")]
async fn refresh_rotates_session_and_revokes_old_token(pool: PgPool) {
    let app = app(pool);
    let auth = register_ok(&app, "rotator", "rotate@example.com").await;
    let old = session_token(&auth).to_string();

    // The old token works before refresh.
    assert_eq!(get(&app, "/me", Some(&old)).await.status(), StatusCode::OK);

    // Refresh issues a NEW session token (and a fresh minecraft pair).
    let refreshed = body_json(post(&app, "/refresh", Some(&old), json!({})).await).await;
    let new = session_token(&refreshed).to_string();
    assert_ne!(new, old, "refresh rotates to a new token");
    assert!(refreshed["minecraft"]["accessToken"].as_str().is_some());
    assert_eq!(refreshed["profile"]["id"], auth["profile"]["id"]);

    // The NEW token authorizes /me; the OLD token is now revoked → 401.
    assert_eq!(get(&app, "/me", Some(&new)).await.status(), StatusCode::OK);
    assert_eq!(
        get(&app, "/me", Some(&old)).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // Refresh without any bearer is unauthorized.
    assert_eq!(
        post(&app, "/refresh", None, json!({})).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn logout_revokes_the_presenting_token(pool: PgPool) {
    let app = app(pool);
    let auth = register_ok(&app, "logouter", "logout@example.com").await;
    let token = session_token(&auth).to_string();

    // The token works before logout.
    assert_eq!(
        get(&app, "/me", Some(&token)).await.status(),
        StatusCode::OK
    );

    // Logout acks and revokes the presenting token.
    let resp = post(&app, "/logout", Some(&token), json!({})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["ok"], true);

    // The token no longer authorizes.
    assert_eq!(
        get(&app, "/me", Some(&token)).await.status(),
        StatusCode::UNAUTHORIZED
    );
}
