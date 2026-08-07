//! Retarget semantics for a build's owned bundle, driven through the admin
//! `PATCH /clients/{id}` route.
//!
//! BUG-5 (P0 data-safety): an EXPLICIT retarget must tear the previously-provisioned
//! bundle down — the `bundle_id ON DELETE SET NULL` FK would otherwise orphan the old
//! `bundles` row, its `bundle_artifacts`, and the on-disk `builds/{oldSlug}/` tree
//! forever. A shared (still-referenced) bundle is always left intact.
//!
//! BEQ-12: destruction is opt-in. An ABSENT `bundleSlug` keeps the existing link, so a
//! pure slug rename retargets nothing; and a stranded bundle that still holds artifacts
//! survives unless the caller sends `deleteOrphanedBundle: true`, coming back as
//! `orphanedBundleSlug` instead.

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

/// Create a build named `slug` (auto-provisioning its 1:1 owned bundle + dir), then
/// seed one artifact row and its on-disk file so teardown/survival is observable.
async fn seed_build_with_one_artifact(
    pool: &PgPool,
    root: &TempRoot,
    token: &str,
    slug: &str,
) -> (Uuid, Uuid, std::path::PathBuf) {
    let body = json!({
        "slug": slug,
        "available": true,
        "locales": [{ "locale": "en", "title": "Old" }],
    });
    let (status, res) = admin_request(
        state_with_root(pool.clone(), root),
        "POST",
        "/clients",
        token,
        Some(&body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let client_id = Uuid::parse_str(res.get("id").and_then(Value::as_str).unwrap()).unwrap();

    let bundle_id: Uuid = sqlx::query_scalar("SELECT id FROM bundles WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("owned bundle row exists");

    sqlx::query(
        "INSERT INTO bundle_artifacts (bundle_id, relative_path, name, category, size, is_dir) \
         VALUES ($1, 'mods/a.jar', 'a.jar', 'mods', 10, false)",
    )
    .bind(bundle_id)
    .execute(pool)
    .await
    .expect("seed artifact");

    let dir = std::path::Path::new(root.as_str())
        .join("builds")
        .join(slug);
    std::fs::create_dir_all(dir.join("files").join("mods")).expect("mkdir files");
    std::fs::write(dir.join("files").join("mods").join("a.jar"), b"hi").expect("write file");
    assert!(dir.exists(), "build dir exists before update");

    (client_id, bundle_id, dir)
}

async fn count_artifacts(pool: &PgPool, bundle_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM bundle_artifacts WHERE bundle_id = $1")
        .bind(bundle_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn count_bundles(pool: &PgPool, bundle_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE id = $1")
        .bind(bundle_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn linked_bundle(pool: &PgPool, client_id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT bundle_id FROM catalog_clients WHERE id = $1")
        .bind(client_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_to_new_slug_retargets_and_tears_down_old_bundle(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let (client_id, old_bundle_id, old_dir) =
        seed_build_with_one_artifact(&pool, &root, &token, "old-slug").await;

    // Rename the build to "new-slug", retargeting EXPLICITLY and opting in to destroying
    // the bundle the move strands (it holds an artifact, so the opt-in is required).
    let update = json!({
        "slug": "new-slug",
        "available": true,
        "bundleSlug": "new-slug",
        "deleteOrphanedBundle": true,
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
    assert_eq!(
        linked_bundle(&pool, client_id).await,
        Some(new_bundle_id),
        "client links to the new bundle"
    );

    assert_eq!(
        count_bundles(&pool, old_bundle_id).await,
        0,
        "old owned bundle row torn down, not orphaned"
    );
    assert_eq!(
        count_artifacts(&pool, old_bundle_id).await,
        0,
        "old bundle artifacts cascade-deleted"
    );
    assert!(!old_dir.exists(), "old on-disk build dir deleted");
}

/// BEQ-12: a slug rename that does NOT mention `bundleSlug` must keep the existing link.
/// Before the fix this silently re-resolved the link from the NEW slug, deleted the old
/// bundle row + artifacts, and rm -rf'd gigabytes of uploads — with a 200 response.
#[sqlx::test(migrations = "../../migrations")]
async fn slug_rename_without_bundle_slug_keeps_the_bundle_and_its_files(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let (client_id, bundle_id, dir) =
        seed_build_with_one_artifact(&pool, &root, &token, "old-slug").await;

    let update = json!({
        "slug": "new-slug",
        "available": true,
        "locales": [{ "locale": "en", "title": "New" }],
    });
    let (status, res) = admin_request(
        state_with_root(pool.clone(), &root),
        "PATCH",
        &format!("/clients/{client_id}"),
        &token,
        Some(&update),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        res["orphanedBundleSlug"].is_null(),
        "nothing was stranded: {res}"
    );

    assert_eq!(
        linked_bundle(&pool, client_id).await,
        Some(bundle_id),
        "the link still points at the build's original bundle"
    );
    assert_eq!(count_bundles(&pool, bundle_id).await, 1, "bundle row kept");
    assert_eq!(count_artifacts(&pool, bundle_id).await, 1, "artifact kept");
    assert!(dir.exists(), "on-disk build dir kept");
    assert!(
        dir.join("files").join("mods").join("a.jar").exists(),
        "uploaded file kept"
    );
    // No bundle was provisioned for the new slug — the rename touched the link at all.
    let new_slug_bundles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bundles WHERE slug = $1")
        .bind("new-slug")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(new_slug_bundles, 0, "no stray bundle for the new slug");
}

/// BEQ-12: an EXPLICIT retarget away from a bundle that still holds artifacts leaves it
/// in place (reported as `orphanedBundleSlug`) unless the caller opts in.
#[sqlx::test(migrations = "../../migrations")]
async fn explicit_retarget_without_opt_in_keeps_a_non_empty_orphan(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let (client_id, old_bundle_id, old_dir) =
        seed_build_with_one_artifact(&pool, &root, &token, "old-slug").await;

    let update = json!({
        "slug": "old-slug",
        "available": true,
        "bundleSlug": "elsewhere",
        "locales": [{ "locale": "en", "title": "Old" }],
    });
    let (status, res) = admin_request(
        state_with_root(pool.clone(), &root),
        "PATCH",
        &format!("/clients/{client_id}"),
        &token,
        Some(&update),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        res["orphanedBundleSlug"], "old-slug",
        "the stranded bundle is reported so an operator can delete it deliberately"
    );

    assert_eq!(
        count_bundles(&pool, old_bundle_id).await,
        1,
        "non-empty stranded bundle survives without the opt-in"
    );
    assert_eq!(count_artifacts(&pool, old_bundle_id).await, 1);
    assert!(old_dir.exists(), "its on-disk files survive too");
}

/// An EMPTY stranded bundle carries nothing to lose, so it is still collected
/// automatically — the opt-in guards uploads, not bookkeeping rows.
#[sqlx::test(migrations = "../../migrations")]
async fn explicit_retarget_collects_an_empty_orphan(pool: PgPool) {
    let root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let body = json!({
        "slug": "empty-build",
        "available": true,
        "locales": [{ "locale": "en", "title": "Empty" }],
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
        .bind("empty-build")
        .fetch_one(&pool)
        .await
        .unwrap();

    let update = json!({
        "slug": "empty-build",
        "available": true,
        "bundleSlug": "somewhere-else",
        "locales": [{ "locale": "en", "title": "Empty" }],
    });
    let (status, res) = admin_request(
        state_with_root(pool.clone(), &root),
        "PATCH",
        &format!("/clients/{client_id}"),
        &token,
        Some(&update),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        res["orphanedBundleSlug"].is_null(),
        "collected, not reported"
    );
    assert_eq!(count_bundles(&pool, old_bundle_id).await, 0);
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
