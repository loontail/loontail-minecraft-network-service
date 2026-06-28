//! BUG-5 (P0 data-safety): updating a build to a new slug re-points its owned bundle
//! and must tear down the previously-provisioned bundle — the `bundle_id ON DELETE SET
//! NULL` FK would otherwise orphan the old `bundles` row, its `bundle_artifacts`, and
//! the on-disk `builds/{oldSlug}/` tree forever. These tests drive the admin
//! `PATCH /clients/{id}` route and assert the new bundle is linked while the old one is
//! gone, and that a shared (still-referenced) bundle is left intact.

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

/// A throwaway storage root under the system temp dir so the test can observe the real
/// on-disk `builds/{slug}` tree being created and then deleted.
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
async fn update_to_new_slug_retargets_and_tears_down_old_bundle(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;

    // Create a build under "old-slug" — auto-provisions its 1:1 owned bundle + dir.
    let body = json!({
        "slug": "old-slug",
        "available": true,
        "locales": [{ "locale": "en", "title": "Old" }],
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

    let old_bundle_id: Uuid = sqlx::query_scalar("SELECT id FROM bundles WHERE slug = $1")
        .bind("old-slug")
        .fetch_one(&pool)
        .await
        .expect("old owned bundle row exists");

    // Seed an artifact row + on-disk file so we can prove both are torn down on retarget.
    sqlx::query(
        "INSERT INTO bundle_artifacts (bundle_id, relative_path, name, category, size, is_dir) \
         VALUES ($1, 'mods/a.jar', 'a.jar', 'mods', 10, false)",
    )
    .bind(old_bundle_id)
    .execute(&pool)
    .await
    .expect("seed artifact");

    let old_dir = std::path::Path::new(root.as_str())
        .join("builds")
        .join("old-slug");
    std::fs::create_dir_all(old_dir.join("files").join("mods")).expect("mkdir files");
    std::fs::write(old_dir.join("files").join("mods").join("a.jar"), b"hi").expect("write file");
    assert!(old_dir.exists(), "old build dir exists before update");

    // Rename the build to "new-slug".
    let update = json!({
        "slug": "new-slug",
        "available": true,
        "locales": [{ "locale": "en", "title": "New" }],
    });
    let (status, _) = admin_request(
        state_with_root(pool.clone(), &root),
        "PATCH",
        &format!("/clients/{client_id}"),
        &token,
        Some(&update),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A bundle for "new-slug" now exists and the client links to it.
    let new_bundle_id: Uuid = sqlx::query_scalar("SELECT id FROM bundles WHERE slug = $1")
        .bind("new-slug")
        .fetch_one(&pool)
        .await
        .expect("new owned bundle row exists");
    let linked: Option<Uuid> =
        sqlx::query_scalar("SELECT bundle_id FROM catalog_clients WHERE id = $1")
            .bind(client_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        linked,
        Some(new_bundle_id),
        "client links to the new bundle"
    );

    // The old bundle row is gone (not orphaned).
    let old_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE id = $1")
        .bind(old_bundle_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(old_count, 0, "old owned bundle row torn down, not orphaned");

    // Its artifacts cascade-deleted.
    let artifact_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bundle_artifacts WHERE bundle_id = $1")
            .bind(old_bundle_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(artifact_count, 0, "old bundle artifacts cascade-deleted");

    // The old on-disk tree is gone.
    assert!(!old_dir.exists(), "old on-disk build dir deleted");
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_keeps_shared_bundle_still_referenced(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;

    // A shared bundle referenced by two builds via explicit bundleSlug.
    let shared_id: Uuid = sqlx::query_scalar(
        "INSERT INTO bundles (slug, name, status) VALUES ('shared', 'Shared', 'draft') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let keep_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients (slug, available, bundle_id, bundle_slug) \
         VALUES ('keeper', true, $1, 'shared') RETURNING id",
    )
    .bind(shared_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let _ = keep_id;

    let mover_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients (slug, available, bundle_id, bundle_slug) \
         VALUES ('mover', true, $1, 'shared') RETURNING id",
    )
    .bind(shared_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Move `mover` off the shared bundle onto its own.
    let update = json!({
        "slug": "mover",
        "available": true,
        "bundleSlug": "mover-own",
        "locales": [{ "locale": "en", "title": "Mover" }],
    });
    let (status, _) = admin_request(
        state_with_root(pool.clone(), &root),
        "PATCH",
        &format!("/clients/{mover_id}"),
        &token,
        Some(&update),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The shared bundle survives because `keeper` still links to it.
    let shared_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE id = $1")
        .bind(shared_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(shared_count, 1, "shared bundle kept while still referenced");
}
