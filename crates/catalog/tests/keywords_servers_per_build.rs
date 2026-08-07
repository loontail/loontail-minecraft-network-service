//! Wave B: keywords/servers are managed per-build. Created keywords/servers are
//! published on insert (so the catalog read, which inlines a client's keyword/server
//! only when `published_at IS NOT NULL`, surfaces them immediately), the detach
//! endpoints drop the join row idempotently, and `update_client` re-links the owned
//! bundle the same way create does (giving a legacy null-bundle build a bundle).

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

async fn admin_send(
    pool: PgPool,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let app = loontail_catalog::admin_routes().with_state(state(pool));
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            builder
                .body(Body::from(serde_json::to_vec(value).unwrap()))
                .unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = app.oneshot(request).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_keyword_and_server_are_published(pool: PgPool) {
    let token = seed_admin_token(&pool).await;

    let (status, kw) = admin_send(
        pool.clone(),
        "POST",
        "/keywords",
        &token,
        Some(&json!({ "slug": "tech", "locales": [{ "locale": "en", "title": "Tech" }] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let kw_id = Uuid::parse_str(kw.get("id").and_then(Value::as_str).unwrap()).unwrap();

    let kw_published: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM catalog_keywords WHERE id = $1 AND published_at IS NOT NULL",
    )
    .bind(kw_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(
        kw_published.is_some(),
        "created keyword is published on insert"
    );

    let (status, srv) = admin_send(
        pool.clone(),
        "POST",
        "/servers",
        &token,
        Some(&json!({ "slug": "survival", "name": "Survival", "address": "play.example.net" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let srv_id = Uuid::parse_str(srv.get("id").and_then(Value::as_str).unwrap()).unwrap();

    let srv_published: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM catalog_servers WHERE id = $1 AND published_at IS NOT NULL",
    )
    .bind(srv_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(
        srv_published.is_some(),
        "created server is published on insert"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn detach_keyword_and_server_drop_join_rows_idempotently(pool: PgPool) {
    let token = seed_admin_token(&pool).await;

    // A published client to attach to.
    let client_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients (slug, available, published_at) \
         VALUES ('aurora', true, now()) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let (_, kw) = admin_send(
        pool.clone(),
        "POST",
        "/keywords",
        &token,
        Some(&json!({ "slug": "tech", "locales": [{ "locale": "en", "title": "Tech" }] })),
    )
    .await;
    let kw_id = kw.get("id").and_then(Value::as_str).unwrap().to_string();
    let (_, srv) = admin_send(
        pool.clone(),
        "POST",
        "/servers",
        &token,
        Some(&json!({ "slug": "survival", "address": "play.example.net" })),
    )
    .await;
    let srv_id = srv.get("id").and_then(Value::as_str).unwrap().to_string();

    let cid = client_id.simple().to_string();

    // Attach both, then detach both.
    for uri in [
        format!("/clients/{cid}/keywords/{kw_id}"),
        format!("/clients/{cid}/servers/{srv_id}"),
    ] {
        let (status, _) = admin_send(pool.clone(), "POST", &uri, &token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    let (status, _) = admin_send(
        pool.clone(),
        "DELETE",
        &format!("/clients/{cid}/keywords/{kw_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = admin_send(
        pool.clone(),
        "DELETE",
        &format!("/clients/{cid}/servers/{srv_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let kw_links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM catalog_client_keywords WHERE client_id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let srv_links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM catalog_client_servers WHERE client_id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kw_links, 0, "keyword join row gone");
    assert_eq!(srv_links, 0, "server join row gone");

    // Detaching again is idempotent (204, no error) even though no row remains.
    let (status, _) = admin_send(
        pool.clone(),
        "DELETE",
        &format!("/clients/{cid}/keywords/{kw_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_client_provisions_bundle_for_legacy_null_bundle_build(pool: PgPool) {
    let token = seed_admin_token(&pool).await;

    // A legacy client with NO owned bundle (bundle_id NULL), as pre-Wave-A rows are.
    let client_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients (slug, available, published_at, bundle_id) \
         VALUES ('legacy', true, now(), NULL) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let (status, _) = admin_send(
        pool.clone(),
        "PATCH",
        &format!("/clients/{}", client_id.simple()),
        &token,
        Some(&json!({
            "slug": "legacy",
            "available": true,
            "locales": [{ "locale": "en", "title": "Legacy" }],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The update now gives it an owned bundle linked by the effective (client) slug.
    let (bundle_id, bundle_slug): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT bundle_id, bundle_slug FROM catalog_clients WHERE id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bundle_slug.as_deref(), Some("legacy"));
    let bundle_id = bundle_id.expect("update provisioned a bundle for the legacy build");
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM bundles WHERE id = $1")
        .bind(bundle_id)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(exists.is_some(), "the provisioned bundle row exists");
}
