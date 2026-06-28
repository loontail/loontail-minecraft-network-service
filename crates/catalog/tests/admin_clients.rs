//! Admin catalog list contract: `GET /clients` on `admin_routes()` returns ALL
//! clients including drafts, each with its real `published` flag — the surface
//! the admin SPA needs to manage freshly created (unpublished) builds. The public
//! list (`contract.rs`) keeps the draft filter, so unpublished builds stay hidden
//! from the launcher.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use loontail_core::auth::issue_session;
use loontail_core::identity::register_user;
use loontail_core::{AppState, Config};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

fn state(pool: PgPool) -> AppState {
    let config = Config::from_env().expect("config from env");
    AppState::new(pool, config)
}

/// Register a user, promote it to `is_admin`, and mint a session Bearer token.
async fn seed_admin_token(pool: &PgPool) -> String {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let user = register_user(
        pool,
        &format!("admin-{nonce}"),
        &format!("admin-{nonce}@example.com"),
        "pw",
    )
    .await
    .expect("register admin");
    sqlx::query("UPDATE users SET is_admin = true WHERE id = $1")
        .bind(user.id)
        .execute(pool)
        .await
        .expect("promote admin");
    issue_session(pool, user.id, Duration::from_secs(900))
        .await
        .expect("issue session")
        .token
}

async fn admin_get(pool: PgPool, uri: &str, token: &str) -> (StatusCode, Value) {
    let app = loontail_catalog::admin_routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

async fn insert_client(pool: &PgPool, slug: &str, published: bool) {
    let published_at = if published { "now()" } else { "NULL" };
    sqlx::query(&format!(
        "INSERT INTO catalog_clients (slug, available, published_at) \
         VALUES ($1, true, {published_at})"
    ))
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) \
         SELECT id, 'en', $2 FROM catalog_clients WHERE slug = $1",
    )
    .bind(slug)
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_list_includes_drafts_with_published_flag(pool: PgPool) {
    insert_client(&pool, "live-build", true).await;
    insert_client(&pool, "draft-build", false).await;
    let token = seed_admin_token(&pool).await;

    let (status, body) = admin_get(pool, "/clients?locale=en", &token).await;
    assert_eq!(status, StatusCode::OK);

    let clients = body
        .get("clients")
        .and_then(Value::as_array)
        .expect("clients array");
    assert_eq!(clients.len(), 2, "admin list must include the draft");

    let by_slug = |slug: &str| -> Value {
        clients
            .iter()
            .find(|c| c.get("slug").and_then(Value::as_str) == Some(slug))
            .unwrap_or_else(|| panic!("{slug} present"))
            .clone()
    };
    let live = by_slug("live-build");
    let draft = by_slug("draft-build");
    assert_eq!(live.get("published").and_then(Value::as_bool), Some(true));
    assert_eq!(draft.get("published").and_then(Value::as_bool), Some(false));

    // Still the native flat shape: no envelope, undashed id, inlined relations.
    assert!(body.get("data").is_none());
    assert_eq!(
        live.get("id").and_then(Value::as_str).map(str::len),
        Some(32)
    );
    assert!(live
        .get("screenshots")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_list_requires_admin(pool: PgPool) {
    // A non-admin session is forbidden; an absent token is unauthorized.
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let user = register_user(
        &pool,
        &format!("plain-{nonce}"),
        &format!("plain-{nonce}@example.com"),
        "pw",
    )
    .await
    .unwrap();
    let token = issue_session(&pool, user.id, Duration::from_secs(900))
        .await
        .unwrap()
        .token;

    let (status, _) = admin_get(pool.clone(), "/clients", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let app = loontail_catalog::admin_routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
