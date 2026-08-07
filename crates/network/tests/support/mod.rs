//! Shared fixtures for the network integration test binaries: building the router
//! over a `#[sqlx::test]` pool, seeding authenticated users, and the small REST
//! helpers both suites drive.
//!
//! why(allow): this module is compiled into each test binary separately, so a
//! fixture used by only one of them would otherwise trip `-D warnings`.
#![allow(dead_code)]

use std::time::Duration;

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

/// State wired to the injected test pool. The pool comes from `#[sqlx::test]`, so
/// the `DATABASE_URL` placeholder is never used.
pub fn state(pool: PgPool) -> AppState {
    // why: only a placeholder when the var is absent — clobbering a real DATABASE_URL
    // trips sqlx::test's "DATABASE_URL changed at runtime" assertion mid-run.
    if std::env::var_os("DATABASE_URL").is_none() {
        std::env::set_var("DATABASE_URL", "postgres://unused");
    }
    let mut config = Config::from_env().unwrap();
    // A generous heartbeat window so freshly-seeded presence rows read as live
    // across the whole test, independent of wall-clock timing.
    config.heartbeat_timeout = Duration::from_secs(3600);
    AppState::new(pool, config)
}

/// The network router with its own fresh state.
pub fn app(pool: PgPool) -> Router {
    loontail_network::routes().with_state(state(pool))
}

pub async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// A seeded network user: its issued session bearer token plus the user id, so
/// tests can reference its identity without a second round-trip.
pub struct TestUser {
    pub token: String,
    pub id: String,
}

/// Mint a session for an account directly via the core functions. The test app
/// builds only `loontail_network::routes()` (no `/api/auth`), so sessions cannot
/// be obtained over HTTP — we register a Yggdrasil account (no minecraft_uuid,
/// random profile_uuid) and issue a session against the pool.
pub async fn mint_session(pool: &PgPool, username: &str) -> (uuid::Uuid, String) {
    let email = format!("{username}@test.invalid");
    let user = loontail_core::identity::register_user(pool, username, &email, "test-password")
        .await
        .expect("register account");
    let session = loontail_core::auth::issue_session(pool, user.id, Duration::from_secs(3600))
        .await
        .expect("issue session");
    (user.id, session.token)
}

/// POST an authenticated `/users/bootstrap` for `user`, binding the live
/// Minecraft identity + presence. Returns the raw response.
pub async fn post_bootstrap(
    app: &Router,
    token: &str,
    minecraft_uuid: &str,
    username: &str,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users/bootstrap")
                .header("content-type", "application/json")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(
                    json!({
                        "minecraftUuid": minecraft_uuid,
                        "username": username,
                        "minecraftVersion": "1.21.4",
                        "loader": "fabric"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Register an account, issue its session, then run the now-authenticated
/// `/users/bootstrap` to bind the live Minecraft identity + mark presence online.
/// Asserts the `{ user }` response shape and returns the session + user id for
/// follow-up authenticated calls.
pub async fn seed_user(
    pool: &PgPool,
    app: &Router,
    minecraft_uuid: &str,
    username: &str,
) -> TestUser {
    let (id, token) = mint_session(pool, username).await;

    let resp = post_bootstrap(app, &token, minecraft_uuid, username).await;
    assert_eq!(resp.status(), StatusCode::OK, "bootstrap should succeed");
    let body = body_json(resp).await;
    assert!(
        body.get("token").is_none(),
        "bootstrap no longer issues a token"
    );
    let user = &body["user"];
    assert_eq!(user["username"], username);
    assert_eq!(user["minecraftUuid"], minecraft_uuid);
    assert_eq!(user["id"].as_str().expect("user id"), id.to_string());
    TestUser {
        token,
        id: id.to_string(),
    }
}

/// Issue an authenticated request as `user`, returning the raw response.
pub async fn authed(
    app: &Router,
    user: &TestUser,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {}", user.token));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

/// Make `a` and `b` friends via the request/accept flow.
pub async fn befriend(app: &Router, a: &TestUser, b: &TestUser) {
    let req = body_json(
        authed(
            app,
            a,
            "POST",
            "/friends/requests",
            Some(json!({ "toUserId": b.id })),
        )
        .await,
    )
    .await;
    let request_id = req["id"].as_str().unwrap();
    let resp = authed(
        app,
        b,
        "POST",
        &format!("/friends/requests/{request_id}/accept"),
        None,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "friend accept should succeed"
    );
}
