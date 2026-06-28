//! Wave A (P0 data-safety): deleting a build (catalog client) must also delete its
//! owned bundle — the `bundle_id ON DELETE SET NULL` FK would otherwise orphan the
//! `bundles` row, its `bundle_artifacts`, and the on-disk `builds/{slug}/` tree
//! forever. These tests drive the admin `DELETE /clients/{id}` route and assert the
//! bundle row + artifacts + directory are gone, and that the create path links a
//! client to its bundle atomically (no stray state).

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

/// A throwaway storage root under the system temp dir so the test can observe the
/// real on-disk `builds/{slug}` tree being created and then deleted.
struct TempRoot {
    path: std::path::PathBuf,
}

impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("llapi-bundles-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp storage root");
        Self { path }
    }

    fn as_str(&self) -> &str {
        self.path.to_str().expect("utf-8 temp path")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn state_with_root(pool: PgPool, root: &TempRoot) -> AppState {
    let mut config = Config::from_env().expect("config from env");
    config.bundles.storage_root = root.as_str().to_string();
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

async fn admin_request(
    state: AppState,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let app = loontail_catalog::admin_routes().with_state(state);
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            builder
                .body(Body::from(serde_json::to_vec(b).unwrap()))
                .unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = app.oneshot(request).await.unwrap();
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
async fn delete_client_removes_owned_bundle_artifacts_and_dir(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;

    // Create a build — this auto-provisions its 1:1 owned bundle and its dir.
    let body = json!({
        "slug": "doomed",
        "available": true,
        "locales": [{ "locale": "en", "title": "Doomed" }],
    });
    let (status, res) = admin_request(
        state_with_root(pool.clone(), &root),
        "POST",
        "/clients",
        &token,
        Some(&body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let client_id = Uuid::parse_str(res.get("id").and_then(Value::as_str).unwrap()).unwrap();

    let bundle_id: Uuid = sqlx::query_scalar("SELECT id FROM bundles WHERE slug = $1")
        .bind("doomed")
        .fetch_one(&pool)
        .await
        .expect("owned bundle row exists");

    // Seed an artifact row + an on-disk file so we can prove both are torn down.
    sqlx::query(
        "INSERT INTO bundle_artifacts (bundle_id, relative_path, name, category, size, is_dir) \
         VALUES ($1, 'mods/a.jar', 'a.jar', 'mods', 10, false)",
    )
    .bind(bundle_id)
    .execute(&pool)
    .await
    .expect("seed artifact");

    let build_dir = std::path::Path::new(root.as_str())
        .join("builds")
        .join("doomed");
    std::fs::create_dir_all(build_dir.join("files").join("mods")).expect("mkdir files");
    std::fs::write(build_dir.join("files").join("mods").join("a.jar"), b"hi").expect("write file");
    assert!(build_dir.exists(), "build dir exists before delete");

    // Delete the build.
    let (status, _) = admin_request(
        state_with_root(pool.clone(), &root),
        "DELETE",
        &format!("/clients/{client_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Client gone.
    let client_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM catalog_clients WHERE id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(client_count, 0, "client row deleted");

    // Bundle row gone (no orphan).
    let bundle_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE id = $1")
        .bind(bundle_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bundle_count, 0, "owned bundle row deleted, not orphaned");

    // Artifacts gone (CASCADE).
    let artifact_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bundle_artifacts WHERE bundle_id = $1")
            .bind(bundle_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(artifact_count, 0, "bundle artifacts cascade-deleted");

    // On-disk tree gone.
    assert!(!build_dir.exists(), "on-disk build dir deleted");
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_legacy_null_bundle_client_just_deletes_client(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;

    // A legacy build with NULL bundle_id (inserted directly, bypassing provisioning).
    let client_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients (slug, available) VALUES ('legacy', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let (status, _) = admin_request(
        state_with_root(pool.clone(), &root),
        "DELETE",
        &format!("/clients/{client_id}"),
        &token,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "null-bundle delete succeeds"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_clients WHERE id = $1")
        .bind(client_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "legacy client deleted");
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_missing_client_is_404(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let (status, _) = admin_request(
        state_with_root(pool.clone(), &root),
        "DELETE",
        &format!("/clients/{}", Uuid::new_v4()),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_links_client_and_bundle_atomically(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;

    let body = json!({
        "slug": "atomic",
        "available": true,
        "locales": [{ "locale": "en", "title": "Atomic" }],
    });
    let (status, res) = admin_request(
        state_with_root(pool.clone(), &root),
        "POST",
        "/clients",
        &token,
        Some(&body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let client_id = Uuid::parse_str(res.get("id").and_then(Value::as_str).unwrap()).unwrap();

    // The client links to exactly one bundle, the bundle exists, and its slug matches
    // — no half-provisioned (client-without-bundle) state.
    let (linked, slug): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT bundle_id, bundle_slug FROM catalog_clients WHERE id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let bundle_id = linked.expect("client linked to a bundle");
    assert_eq!(slug.as_deref(), Some("atomic"));

    let bundle_slug: String = sqlx::query_scalar("SELECT slug FROM bundles WHERE id = $1")
        .bind(bundle_id)
        .fetch_one(&pool)
        .await
        .expect("bundle row exists for linked id");
    assert_eq!(bundle_slug, "atomic");

    // The on-disk dir was created after commit.
    let files_dir = std::path::Path::new(root.as_str())
        .join("builds")
        .join("atomic")
        .join("files");
    assert!(files_dir.exists(), "bundle files dir created after commit");
}

#[sqlx::test(migrations = "../../migrations")]
async fn create_rolls_back_client_when_bundle_provision_fails(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;

    // The client slug is valid, but the explicit bundleSlug is not a single inert path
    // segment, so `provision_bundle_row`'s slug validation fails *after* the client row
    // has been inserted in the shared tx. The whole create must roll back: no client
    // and no bundle should persist.
    let body = json!({
        "slug": "rollback-me",
        "available": true,
        "bundleSlug": "bad/slug",
        "locales": [{ "locale": "en", "title": "Rollback" }],
    });
    let (status, _) = admin_request(
        state_with_root(pool.clone(), &root),
        "POST",
        "/clients",
        &token,
        Some(&body),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::CREATED,
        "create with an invalid bundle slug must not succeed"
    );

    // The client row was rolled back with the failed provision (shared tx).
    let client_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM catalog_clients WHERE slug = $1")
            .bind("rollback-me")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        client_count, 0,
        "client row rolled back on provision failure"
    );

    let bundle_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE slug = $1")
        .bind("bad/slug")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bundle_count, 0, "no bundle row persisted");
}
