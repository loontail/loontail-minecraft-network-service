//! Wave 2: creating a catalog build auto-provisions its owned bundle (1:1 model)
//! and links it, so a new build immediately has a resolvable manifest URL. These
//! tests drive the admin `POST /clients` route and assert the bundle row + link,
//! the `publish: true` path is immediately visible on the public read with an
//! inlined `bundle.manifestUrl`, and an explicit `bundleSlug` reuses a pre-existing
//! bundle rather than creating a duplicate.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use loontail_core::auth::issue_session;
use loontail_core::identity::register_user;
use loontail_core::{AppState, Config};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

fn state(pool: PgPool) -> AppState {
    let config = Config::from_env().expect("config from env");
    AppState::new(pool, config)
}

async fn seed_admin_token(pool: &PgPool) -> String {
    let nonce = Uuid::new_v4().simple().to_string();
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

async fn seed_session_token(pool: &PgPool) -> String {
    let nonce = Uuid::new_v4().simple().to_string();
    let user = register_user(
        pool,
        &format!("reader-{nonce}"),
        &format!("reader-{nonce}@example.com"),
        "pw",
    )
    .await
    .expect("register reader");
    issue_session(pool, user.id, Duration::from_secs(900))
        .await
        .expect("issue session")
        .token
}

/// POST a JSON body to the admin `routes()` and return `(status, json)`.
async fn admin_post(pool: PgPool, uri: &str, token: &str, body: &Value) -> (StatusCode, Value) {
    let app = loontail_catalog::admin_routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(body).unwrap()))
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

/// GET from the public `routes()` (AuthUser-guarded) and return `(status, json)`.
async fn public_get(pool: PgPool, uri: &str, token: &str) -> (StatusCode, Value) {
    let app = loontail_catalog::routes().with_state(state(pool));
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

#[sqlx::test(migrations = "../../migrations")]
async fn create_without_bundle_slug_provisions_owned_bundle(pool: PgPool) {
    let token = seed_admin_token(&pool).await;
    let body = json!({
        "slug": "aurora",
        "available": true,
        "locales": [{ "locale": "en", "title": "Aurora" }],
    });

    let (status, res) = admin_post(pool.clone(), "/clients", &token, &body).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        res.get("bundleSlug").and_then(Value::as_str),
        Some("aurora"),
        "response bundleSlug defaults to the client slug"
    );
    let client_id = Uuid::parse_str(res.get("id").and_then(Value::as_str).unwrap()).unwrap();

    // A bundles row with that slug now exists.
    let bundle_id: Uuid = sqlx::query_scalar("SELECT id FROM bundles WHERE slug = $1")
        .bind("aurora")
        .fetch_one(&pool)
        .await
        .expect("owned bundle row exists");

    // The client's bundle_id points at it.
    let linked: Option<Uuid> =
        sqlx::query_scalar("SELECT bundle_id FROM catalog_clients WHERE id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(linked, Some(bundle_id), "client links to the owned bundle");
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_with_publish_is_visible_with_manifest_url(pool: PgPool) {
    let admin = seed_admin_token(&pool).await;
    let body = json!({
        "slug": "nebula",
        "available": true,
        "publish": true,
        "locales": [{ "locale": "en", "title": "Nebula" }],
    });

    let (status, _res) = admin_post(pool.clone(), "/clients", &admin, &body).await;
    assert_eq!(status, StatusCode::CREATED);

    // The published build is immediately visible via the public read...
    let reader = seed_session_token(&pool).await;
    let (status, list) = public_get(pool.clone(), "/clients?locale=en", &reader).await;
    assert_eq!(status, StatusCode::OK);

    let clients = list
        .get("clients")
        .and_then(Value::as_array)
        .expect("clients array");
    let client = clients
        .iter()
        .find(|c| c.get("slug").and_then(Value::as_str) == Some("nebula"))
        .expect("published nebula visible on public read");

    // ...with its owned bundle inlined and a frozen manifest URL.
    let bundle = client.get("bundle").expect("bundle field present");
    assert!(bundle.is_object(), "bundle inlined, not null");
    assert_eq!(
        bundle.get("manifestUrl").and_then(Value::as_str),
        Some("/api/bundle-registry/builds/nebula/manifest"),
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_with_existing_bundle_slug_links_without_duplicate(pool: PgPool) {
    // Pre-existing bundle the new client should link to (no duplicate row).
    let existing: Uuid = sqlx::query_scalar(
        "INSERT INTO bundles (slug, name, status) VALUES ('shared-pack', 'Shared', 'draft') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let token = seed_admin_token(&pool).await;
    let body = json!({
        "slug": "comet",
        "available": true,
        "bundleSlug": "shared-pack",
        "locales": [{ "locale": "en", "title": "Comet" }],
    });

    let (status, res) = admin_post(pool.clone(), "/clients", &token, &body).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        res.get("bundleSlug").and_then(Value::as_str),
        Some("shared-pack"),
    );
    let client_id = Uuid::parse_str(res.get("id").and_then(Value::as_str).unwrap()).unwrap();

    // No duplicate bundle row for that slug.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE slug = $1")
        .bind("shared-pack")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "links to the existing bundle, no duplicate");

    // The client links to the pre-existing bundle id.
    let linked: Option<Uuid> =
        sqlx::query_scalar("SELECT bundle_id FROM catalog_clients WHERE id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(linked, Some(existing));
}
