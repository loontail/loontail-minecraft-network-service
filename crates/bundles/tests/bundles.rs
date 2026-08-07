//! Integration tests for the bundle registry: ZIP ingest, on-disk layout,
//! artifact rows + sha256, manifest byte-shape, zip-slip rejection, and the
//! public manifest contract (served verbatim from artifacts.json).

use std::io::Write;
use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use loontail_core::auth::issue_session;
use loontail_core::config::Config;
use loontail_core::identity::{admin_create_user, AdminCreateUser};
use loontail_core::AppState;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::util::ServiceExt;

const STORAGE_PUBLIC_PREFIX: &str = "/bundle-registry";

/// Build an `AppState` whose bundle storage root points at a fresh tempdir.
fn test_state(pool: PgPool, storage_root: &Path) -> AppState {
    // why: only a placeholder when the var is absent — clobbering a real DATABASE_URL
    // trips sqlx::test's "DATABASE_URL changed at runtime" assertion mid-run.
    if std::env::var_os("DATABASE_URL").is_none() {
        std::env::set_var("DATABASE_URL", "postgres://unused");
    }
    let mut config = Config::from_env().expect("config");
    config.bundles.storage_root = storage_root.to_string_lossy().into_owned();
    config.bundles.public_url = STORAGE_PUBLIC_PREFIX.to_string();
    AppState::new(pool, config)
}

/// Seed an admin and mint a session, returning its raw token. Sent as
/// `Authorization: Bearer` below — a programmatic admin caller, so CSRF-exempt.
async fn seed_admin_token(pool: &PgPool) -> String {
    let admin = admin_create_user(
        pool,
        AdminCreateUser {
            username: "bundleadmin".into(),
            email: "bundleadmin@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: true,
        },
    )
    .await
    .expect("admin");
    let session = issue_session(pool, admin.id, std::time::Duration::from_secs(900))
        .await
        .expect("session");
    session.token
}

/// Seed a plain (non-admin) user and mint a session token — proves the public
/// reads are gated on `AuthUser` (any session), not `AdminUser`.
async fn seed_user_token(pool: &PgPool) -> String {
    let user = admin_create_user(
        pool,
        AdminCreateUser {
            username: "bundleuser".into(),
            email: "bundleuser@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .expect("user");
    let session = issue_session(pool, user.id, std::time::Duration::from_secs(900))
        .await
        .expect("session");
    session.token
}

/// A small ZIP with files in nested directories, built in memory.
fn build_test_zip() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("mods/alpha.jar", opts).unwrap();
        zip.write_all(b"alpha-bytes").unwrap();

        zip.start_file("mods/sub/beta.jar", opts).unwrap();
        zip.write_all(b"beta-content-here").unwrap();

        zip.start_file("config/settings.cfg", opts).unwrap();
        zip.write_all(b"key=value").unwrap();

        // A __MACOSX entry that must be skipped.
        zip.start_file("__MACOSX/junk", opts).unwrap();
        zip.write_all(b"ignore-me").unwrap();

        zip.finish().unwrap();
    }
    buf
}

/// Build an in-memory ZIP from `(path, contents)` pairs, Stored (uncompressed).
fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (path, contents) in entries {
            zip.start_file(*path, opts).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn multipart_zip_body(zip: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----boundaryBUNDLE";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"archive\"; filename=\"bundle.zip\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/zip\r\n\r\n");
    body.extend_from_slice(zip);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn create_build(state: &AppState, token: &str, slug: &str) {
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let body = serde_json::json!({ "name": "Test Bundle", "slug": slug }).to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/builds")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "create build");
}

async fn upload_zip(state: &AppState, token: &str, slug: &str, zip: &[u8]) -> StatusCode {
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let (content_type, body) = multipart_zip_body(zip);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/upload"))
                .header("content-type", content_type)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_lays_out_files_rows_and_manifest(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "my-build";

    create_build(&state, &token, slug).await;
    let status = upload_zip(&state, &token, slug, &build_test_zip()).await;
    assert_eq!(status, StatusCode::OK, "upload archive");

    // Files land at storage_root/builds/{slug}/files/{relativePath} EXACTLY.
    let files_root = tmp.path().join("builds").join(slug).join("files");
    assert_eq!(
        std::fs::read(files_root.join("mods").join("alpha.jar")).unwrap(),
        b"alpha-bytes"
    );
    assert_eq!(
        std::fs::read(files_root.join("mods").join("sub").join("beta.jar")).unwrap(),
        b"beta-content-here"
    );
    assert_eq!(
        std::fs::read(files_root.join("config").join("settings.cfg")).unwrap(),
        b"key=value"
    );
    // __MACOSX must have been skipped.
    assert!(!files_root.join("__MACOSX").exists());

    // Artifact rows: three files + their directory rows, sha256 correct.
    let alpha_sha: Option<String> = sqlx::query_scalar(
        "SELECT a.sha256 FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = 'mods/alpha.jar'",
    )
    .bind(slug)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alpha_sha, Some(sha256_hex(b"alpha-bytes")));

    let beta_sha: Option<String> = sqlx::query_scalar(
        "SELECT a.sha256 FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = 'mods/sub/beta.jar'",
    )
    .bind(slug)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(beta_sha, Some(sha256_hex(b"beta-content-here")));

    let file_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.is_dir = false",
    )
    .bind(slug)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(file_count, 3, "three files tracked");

    // Bundle bookkeeping: status ready, files_count = 3, total_size = sum.
    let (status_col, files_count_col, total_size_col): (String, i32, i64) =
        sqlx::query_as("SELECT status, files_count, total_size FROM bundles WHERE slug = $1")
            .bind(slug)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status_col, "ready");
    assert_eq!(files_count_col, 3);
    let expected_total =
        (b"alpha-bytes".len() + b"beta-content-here".len() + b"key=value".len()) as i64;
    assert_eq!(total_size_col, expected_total);

    // Manifest on disk: byte-shape checks.
    let manifest_path = tmp.path().join("builds").join(slug).join("artifacts.json");
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

    // Grouped by top-level dir.
    assert!(json.get("mods").is_some(), "grouped by 'mods'");
    assert!(json.get("config").is_some(), "grouped by 'config'");

    // A file entry carries sha256 + url; a dir entry omits both.
    let mods = json["mods"].as_array().unwrap();
    let file_entry = mods
        .iter()
        .find(|e| e["path"] == "mods/alpha.jar")
        .expect("alpha entry");
    assert_eq!(file_entry["sha256"], sha256_hex(b"alpha-bytes"));
    assert_eq!(
        file_entry["url"],
        "/bundle-registry/builds/my-build/files/mods/alpha.jar"
    );
    assert_eq!(file_entry["isDir"], false);
    // downloadOnce omitted when false.
    assert!(file_entry.get("downloadOnce").is_none());

    let dir_entry = mods
        .iter()
        .find(|e| e["isDir"] == true)
        .expect("a directory entry under mods");
    assert!(dir_entry.get("sha256").is_none(), "dirs omit sha256");
    assert!(dir_entry.get("url").is_none(), "dirs omit url");

    // 2-space pretty-printed (the launcher hashes the raw bytes).
    assert!(raw.contains("{\n  \""), "two-space pretty JSON");
}

#[sqlx::test(migrations = "../../migrations")]
async fn public_manifest_is_served_verbatim(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "contract-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // The bytes on disk are the source of truth.
    let manifest_path = tmp.path().join("builds").join(slug).join("artifacts.json");
    let on_disk = std::fs::read(&manifest_path).unwrap();

    let app = loontail_bundles::routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/builds/{slug}/manifest"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "no-cache",
        "manifest is no-cache"
    );
    let served = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(served.as_ref(), on_disk.as_slice(), "served verbatim");
}

#[sqlx::test(migrations = "../../migrations")]
async fn static_route_serves_file_bytes(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "static-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    let app = loontail_bundles::static_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/builds/{slug}/files/config/settings.cfg"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"key=value");
}

#[sqlx::test(migrations = "../../migrations")]
async fn zip_slip_archive_is_rejected(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "evil-build";

    create_build(&state, &token, slug).await;

    // A malicious ZIP whose entry escapes the build directory.
    let mut zip_bytes = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut zip_bytes);
        let mut zip = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("../../escape.txt", opts).unwrap();
        zip.write_all(b"pwned").unwrap();
        zip.finish().unwrap();
    }

    let status = upload_zip(&state, &token, slug, &zip_bytes).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "zip-slip archive must be rejected"
    );

    // The build is marked failed and nothing escaped onto disk.
    let build_status: String = sqlx::query_scalar("SELECT status FROM bundles WHERE slug = $1")
        .bind(slug)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(build_status, "failed");
    assert!(!tmp.path().join("escape.txt").exists());
    assert!(!tmp.path().parent().unwrap().join("escape.txt").exists());
}

#[sqlx::test(migrations = "../../migrations")]
async fn toggle_download_once_reflects_in_manifest(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "once-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    let entry_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT a.id FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = 'mods/alpha.jar'",
    )
    .bind(slug)
    .fetch_one(&pool)
    .await
    .unwrap();

    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/builds/{slug}/files/{entry_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({ "downloadOnce": true }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let manifest_path = tmp.path().join("builds").join(slug).join("artifacts.json");
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let entry = json["mods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "mods/alpha.jar")
        .unwrap();
    assert_eq!(entry["downloadOnce"], true);
}

/// `upsert_artifact` is an `INSERT ... ON CONFLICT DO UPDATE` that deliberately omits
/// `download_once` from the update set — so a re-upload (rescan) of an existing path
/// must NOT reset an operator's `downloadOnce = true` toggle.
#[sqlx::test(migrations = "../../migrations")]
async fn reupload_preserves_download_once(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "preserve-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    let entry_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT a.id FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = 'mods/alpha.jar'",
    )
    .bind(slug)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Toggle download_once = true via the admin route.
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/builds/{slug}/files/{entry_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({ "downloadOnce": true }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Re-upload the same archive — the existing artifact row is upserted (ON CONFLICT).
    assert_eq!(
        upload_zip(&state, &token, slug, &build_test_zip()).await,
        StatusCode::OK
    );

    let download_once: bool =
        sqlx::query_scalar("SELECT download_once FROM bundle_artifacts WHERE id = $1")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        download_once,
        "re-upload preserved the download_once toggle"
    );
}

/// BUG-6 + BUG-7 regression: a full archive re-upload is a REPLACE. Files dropped
/// from the new ZIP must vanish from both disk and the artifact rows; files added must
/// appear; survivors stay. After uploading B over A, exactly B's set remains and the
/// served manifest reflects only B (the manifest is filtered to on-disk files, which
/// after this fix are exactly B).
#[sqlx::test(migrations = "../../migrations")]
async fn reupload_replaces_disk_and_rows(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "replace-build";

    create_build(&state, &token, slug).await;

    // Archive A: a.txt + b.txt.
    let zip_a = build_zip(&[("a.txt", b"alpha"), ("b.txt", b"bravo")]);
    assert_eq!(
        upload_zip(&state, &token, slug, &zip_a).await,
        StatusCode::OK
    );

    let files_root = tmp.path().join("builds").join(slug).join("files");
    assert_eq!(std::fs::read(files_root.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(files_root.join("b.txt")).unwrap(), b"bravo");

    let count_path = |path: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
                 WHERE b.slug = $1 AND a.relative_path = $2",
            )
            .bind(slug)
            .bind(path)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    assert_eq!(count_path("a.txt").await, 1, "a.txt row after A");
    assert_eq!(count_path("b.txt").await, 1, "b.txt row after A");

    // Archive B: a.txt (changed) + c.txt — b.txt is gone, c.txt is new.
    let zip_b = build_zip(&[("a.txt", b"alpha2"), ("c.txt", b"charlie")]);
    assert_eq!(
        upload_zip(&state, &token, slug, &zip_b).await,
        StatusCode::OK
    );

    // Disk: b.txt gone, c.txt present, a.txt updated.
    assert!(!files_root.join("b.txt").exists(), "b.txt file removed");
    assert_eq!(std::fs::read(files_root.join("a.txt")).unwrap(), b"alpha2");
    assert_eq!(std::fs::read(files_root.join("c.txt")).unwrap(), b"charlie");

    // Rows: b.txt row gone, a.txt + c.txt present.
    assert_eq!(count_path("b.txt").await, 0, "b.txt row removed (BUG-6)");
    assert_eq!(count_path("a.txt").await, 1, "a.txt row kept");
    assert_eq!(count_path("c.txt").await, 1, "c.txt row added");

    // Exactly B's file set remains (root files only — no dir rows for top-level files).
    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT a.relative_path FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.is_dir = false ORDER BY a.relative_path",
    )
    .bind(slug)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, vec!["a.txt".to_string(), "c.txt".to_string()]);

    // Manifest contract: reflects only B's files, none of A's dropped files.
    let json = read_manifest(tmp.path(), slug);
    let root = json["root"].as_array().unwrap();
    let paths: Vec<&str> = root.iter().filter_map(|e| e["path"].as_str()).collect();
    assert!(paths.contains(&"a.txt"), "manifest lists a.txt");
    assert!(paths.contains(&"c.txt"), "manifest lists c.txt");
    assert!(!paths.contains(&"b.txt"), "manifest drops b.txt");
}

#[sqlx::test(migrations = "../../migrations")]
async fn public_reads_require_a_session(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "gated-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // Manifest without a token: 401.
    let app = loontail_bundles::routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/builds/{slug}/manifest"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "manifest needs auth"
    );

    // File without a token: 401.
    let app = loontail_bundles::static_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/builds/{slug}/files/config/settings.cfg"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "file needs auth");

    // A plain (non-admin) user session is sufficient for both reads.
    let user_token = seed_user_token(&pool).await;
    let app = loontail_bundles::static_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/builds/{slug}/files/config/settings.cfg"))
                .header("authorization", format!("Bearer {user_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "any session may read files");
}

/// S1 regression: a percent-encoded traversal slug must not escape
/// `{storage_root}/builds`. axum decodes `Path` params after routing, so
/// `..%2F..%2F<name>` arrives as `slug = "../../<name>"`. The handler must reject
/// it (400) or 404 before touching the filesystem — never serve the escaped file.
#[sqlx::test(migrations = "../../migrations")]
async fn slug_path_traversal_is_blocked(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;

    // Plant a secret file ABOVE the builds root that a traversal would expose.
    let builds_root = tmp.path().join("builds");
    std::fs::create_dir_all(&builds_root).unwrap();
    std::fs::write(tmp.path().join("secret.txt"), b"top-secret").unwrap();

    let app = loontail_bundles::routes().with_state(state.clone());

    // Manifest endpoint: `slug = "../../secret"` (no such build) must not 200.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/builds/..%2F..%2Fsecret/manifest")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "traversal slug must not resolve a manifest"
    );

    // File endpoint: try to read `../../secret.txt` via a traversal slug.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/builds/..%2F..%2Fsecret.txt/files/x")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "traversal slug must not serve a file outside builds/"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_ne!(
        body.as_ref(),
        b"top-secret",
        "the secret file must never be served"
    );
}

/// CB-3 regression: the traversal guard is now structural — every admin route that
/// takes `{slug}` resolves it through the `ResolvedBundle`/`ResolvedEntry` extractor,
/// which runs `validate_slug` before any filesystem join. A percent-encoded traversal
/// slug must be a 400 on EVERY such route, not just the ones that remembered the call.
#[sqlx::test(migrations = "../../migrations")]
async fn admin_slug_routes_reject_a_traversal_slug(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;

    std::fs::create_dir_all(tmp.path().join("builds")).unwrap();
    std::fs::write(tmp.path().join("secret.txt"), b"top-secret").unwrap();

    let traversal = "..%2F..%2Fsecret";
    let entry_id = uuid::Uuid::new_v4();
    let routes: &[(&str, String)] = &[
        ("GET", format!("/builds/{traversal}")),
        ("PUT", format!("/builds/{traversal}")),
        ("DELETE", format!("/builds/{traversal}")),
        ("POST", format!("/builds/{traversal}/regenerate")),
        ("POST", format!("/builds/{traversal}/validate")),
        ("POST", format!("/builds/{traversal}/folders")),
        ("POST", format!("/builds/{traversal}/files/bulk-delete")),
        ("POST", format!("/builds/{traversal}/files/move")),
        ("DELETE", format!("/builds/{traversal}/files/{entry_id}")),
        ("PUT", format!("/builds/{traversal}/files/{entry_id}")),
        (
            "POST",
            format!("/builds/{traversal}/files/{entry_id}/rename"),
        ),
        ("POST", format!("/builds/{traversal}/files/{entry_id}/move")),
        (
            "POST",
            format!("/builds/{traversal}/files/{entry_id}/rehash"),
        ),
    ];

    for (method, uri) in routes {
        let app = loontail_bundles::admin_routes().with_state(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(*method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::json!({
                            "relativePath": "x",
                            "ids": [entry_id],
                            "targetDir": "",
                            "newRelativePath": "x",
                            "downloadOnce": true,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{method} {uri} must reject a traversal slug"
        );
    }

    assert!(
        tmp.path().join("secret.txt").exists(),
        "the file above the builds root must be untouched"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_routes_require_admin(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());

    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/builds")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ----- Wave D: file-manager move endpoints -----

/// Look up an artifact id by its exact `relative_path` within a build.
async fn artifact_id(pool: &PgPool, slug: &str, relative_path: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT a.id FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = $2",
    )
    .bind(slug)
    .bind(relative_path)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `category` column for an artifact path (the manifest grouping key).
async fn artifact_category(pool: &PgPool, slug: &str, relative_path: &str) -> String {
    sqlx::query_scalar(
        "SELECT a.category FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = $2",
    )
    .bind(slug)
    .bind(relative_path)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn read_manifest(tmp: &Path, slug: &str) -> serde_json::Value {
    let manifest_path = tmp.join("builds").join(slug).join("artifacts.json");
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// POST a JSON body to an admin route, returning the response status + body bytes.
async fn admin_post_json(
    state: &AppState,
    token: &str,
    uri: &str,
    body: serde_json::Value,
) -> StatusCode {
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

/// Single move across a top-level dir updates `category`, moves the file on disk, and
/// the regenerated manifest reflects the new path under the new category group.
#[sqlx::test(migrations = "../../migrations")]
async fn single_move_updates_category_and_manifest(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "move-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // mods/alpha.jar -> config/alpha.jar (category mods -> config).
    let id = artifact_id(&pool, slug, "mods/alpha.jar").await;
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/{id}/move"),
        serde_json::json!({ "targetDir": "config" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "move should succeed");

    // Row path + category re-derived.
    assert_eq!(
        artifact_category(&pool, slug, "config/alpha.jar").await,
        "config"
    );
    // File physically moved.
    let files_root = tmp.path().join("builds").join(slug).join("files");
    assert!(files_root.join("config").join("alpha.jar").exists());
    assert!(!files_root.join("mods").join("alpha.jar").exists());

    // Manifest reflects the new path under the new category group.
    let json = read_manifest(tmp.path(), slug);
    let config = json["config"].as_array().unwrap();
    assert!(
        config.iter().any(|e| e["path"] == "config/alpha.jar"),
        "manifest lists the moved file under config"
    );
    let mods = json["mods"].as_array().unwrap();
    assert!(
        !mods.iter().any(|e| e["path"] == "mods/alpha.jar"),
        "manifest no longer lists the file under mods"
    );
}

/// Multi-move of N entries into one target dir in a single request; one manifest regen.
#[sqlx::test(migrations = "../../migrations")]
async fn multi_move_relocates_all_entries(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "multi-move-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // Move mods/alpha.jar AND config/settings.cfg into a fresh "vault" dir.
    let alpha = artifact_id(&pool, slug, "mods/alpha.jar").await;
    let cfg = artifact_id(&pool, slug, "config/settings.cfg").await;
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/move"),
        serde_json::json!({ "ids": [alpha, cfg], "targetDir": "vault" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "multi-move should succeed");

    assert_eq!(
        artifact_category(&pool, slug, "vault/alpha.jar").await,
        "vault"
    );
    assert_eq!(
        artifact_category(&pool, slug, "vault/settings.cfg").await,
        "vault"
    );

    let files_root = tmp.path().join("builds").join(slug).join("files");
    assert!(files_root.join("vault").join("alpha.jar").exists());
    assert!(files_root.join("vault").join("settings.cfg").exists());
    assert!(!files_root.join("mods").join("alpha.jar").exists());

    let json = read_manifest(tmp.path(), slug);
    let vault = json["vault"].as_array().unwrap();
    assert!(vault.iter().any(|e| e["path"] == "vault/alpha.jar"));
    assert!(vault.iter().any(|e| e["path"] == "vault/settings.cfg"));
}

/// Moving a folder into its own descendant must be a 4xx (BadRequest), not a 500.
#[sqlx::test(migrations = "../../migrations")]
async fn move_folder_into_own_descendant_is_4xx(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "self-move-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // The "mods" folder row exists (mods/sub/beta.jar implies mods + mods/sub dirs).
    let mods_id = artifact_id(&pool, slug, "mods").await;
    // Try to move "mods" into "mods/sub" -> new path "mods/sub/mods", a descendant.
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/{mods_id}/move"),
        serde_json::json!({ "targetDir": "mods/sub" }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "self-into-descendant must be 4xx, got {status}"
    );
    assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// Moving onto an existing path returns a clean 409 (not a unique-index 500).
#[sqlx::test(migrations = "../../migrations")]
async fn move_onto_existing_path_is_409(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "collide-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // Put a file at config/alpha.jar, then try to move mods/alpha.jar onto config (same name).
    // Upload a second alpha under config so the destination is occupied.
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let boundary = "----b";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"alpha.jar\"\r\n\r\n",
    );
    body.extend_from_slice(b"dupe");
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"targetPath\"\r\n\r\n");
    body.extend_from_slice(b"config/alpha.jar");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/files"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "seed config/alpha.jar");

    // Now move mods/alpha.jar into config -> collides with config/alpha.jar.
    let id = artifact_id(&pool, slug, "mods/alpha.jar").await;
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/{id}/move"),
        serde_json::json!({ "targetDir": "config" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "collision must be 409");

    // The source row is untouched (tx rolled back).
    assert_eq!(
        artifact_category(&pool, slug, "mods/alpha.jar").await,
        "mods"
    );
}

/// Regression: a normal rename (the existing endpoint, now sharing move_subtree) still
/// moves the file + descendants and updates the manifest.
#[sqlx::test(migrations = "../../migrations")]
async fn rename_still_works_after_refactor(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "rename-build";

    create_build(&state, &token, slug).await;
    upload_zip(&state, &token, slug, &build_test_zip()).await;

    // Rename the "mods" folder to "plugins" — children follow.
    let mods_id = artifact_id(&pool, slug, "mods").await;
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/{mods_id}/rename"),
        serde_json::json!({ "newRelativePath": "plugins" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rename should succeed");

    // Folder + descendants rewritten, category re-derived.
    assert_eq!(
        artifact_category(&pool, slug, "plugins/alpha.jar").await,
        "plugins"
    );
    assert_eq!(
        artifact_category(&pool, slug, "plugins/sub/beta.jar").await,
        "plugins"
    );
    let files_root = tmp.path().join("builds").join(slug).join("files");
    assert!(files_root.join("plugins").join("alpha.jar").exists());
    assert!(files_root
        .join("plugins")
        .join("sub")
        .join("beta.jar")
        .exists());
    assert!(!files_root.join("mods").exists());

    let json = read_manifest(tmp.path(), slug);
    let plugins = json["plugins"].as_array().unwrap();
    assert!(plugins.iter().any(|e| e["path"] == "plugins/alpha.jar"));
    assert!(plugins.iter().any(|e| e["path"] == "plugins/sub/beta.jar"));
    assert!(json.get("mods").is_none(), "old category group is gone");
}

/// DELETE an admin route, returning the response status.
async fn admin_delete(state: &AppState, token: &str, uri: &str) -> StatusCode {
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    resp.status()
}

/// `true` when an artifact row exists at `relative_path` for this build.
async fn artifact_row_exists(pool: &PgPool, slug: &str, relative_path: &str) -> bool {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 AND a.relative_path = $2",
    )
    .bind(slug)
    .bind(relative_path)
    .fetch_one(pool)
    .await
    .unwrap();
    count > 0
}

/// Regression (BE-01): `_` is a LIKE wildcard, so deleting `mod_config` must not
/// take the sibling `mod-config` subtree with it.
#[sqlx::test(migrations = "../../migrations")]
async fn folder_delete_does_not_touch_underscore_sibling(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "like-underscore";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[
        ("mod_config/a.cfg", b"a"),
        ("mod-config/b.cfg", b"b"),
        ("mods/keep.jar", b"keep"),
    ]);
    assert_eq!(
        upload_zip(&state, &token, slug, &zip).await,
        StatusCode::OK,
        "upload archive"
    );

    let id = artifact_id(&pool, slug, "mod_config").await;
    assert_eq!(
        admin_delete(&state, &token, &format!("/builds/{slug}/files/{id}")).await,
        StatusCode::OK
    );

    assert!(!artifact_row_exists(&pool, slug, "mod_config/a.cfg").await);
    assert!(
        artifact_row_exists(&pool, slug, "mod-config/b.cfg").await,
        "sibling folder differing only at the `_` position must survive"
    );

    let json = read_manifest(tmp.path(), slug);
    let group = json["mod-config"].as_array().unwrap();
    assert!(group.iter().any(|e| e["path"] == "mod-config/b.cfg"));
    assert!(json.get("mod_config").is_none());
}

/// Regression (BE-01): a folder literally named `%` yields the pattern `%/%`, which
/// unescaped matches every nested path in the bundle.
#[sqlx::test(migrations = "../../migrations")]
async fn folder_named_percent_deletes_only_itself(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "like-percent";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[("%/inside.txt", b"x"), ("mods/a.jar", b"a")]);
    assert_eq!(
        upload_zip(&state, &token, slug, &zip).await,
        StatusCode::OK,
        "upload archive"
    );

    let id = artifact_id(&pool, slug, "%").await;
    assert_eq!(
        admin_delete(&state, &token, &format!("/builds/{slug}/files/{id}")).await,
        StatusCode::OK
    );

    assert!(!artifact_row_exists(&pool, slug, "%/inside.txt").await);
    assert!(
        artifact_row_exists(&pool, slug, "mods/a.jar").await,
        "an unrelated subtree must survive deleting the `%` folder"
    );
}

/// Regression (BE-01): the destination-free check also used an unescaped prefix, so
/// moving into `mod_config` phantom-collided with the existing `mod-config`.
#[sqlx::test(migrations = "../../migrations")]
async fn move_into_dir_differing_by_one_char_is_not_409(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "like-move";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[("mod-config/b.cfg", b"b"), ("stuff/c.cfg", b"c")]);
    assert_eq!(
        upload_zip(&state, &token, slug, &zip).await,
        StatusCode::OK,
        "upload archive"
    );

    let id = artifact_id(&pool, slug, "stuff").await;
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/{id}/rename"),
        serde_json::json!({ "newRelativePath": "mod_config" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "destination is genuinely free");
    assert!(artifact_row_exists(&pool, slug, "mod_config/c.cfg").await);
    assert!(artifact_row_exists(&pool, slug, "mod-config/b.cfg").await);
}

/// Every artifact row for this build, path-ordered.
async fn artifact_paths(pool: &PgPool, slug: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT a.relative_path FROM bundle_artifacts a JOIN bundles b ON b.id = a.bundle_id \
         WHERE b.slug = $1 ORDER BY a.relative_path",
    )
    .bind(slug)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn bundle_row_count(pool: &PgPool, slug: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM bundles WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// BEQ-14: `bulk_delete` used to `continue` past an id belonging to another build after
/// already deleting the earlier ids — a silent partial delete. Every id is resolved up
/// front now, so a FOREIGN id is a clean 404 and NOTHING is removed.
#[sqlx::test(migrations = "../../migrations")]
async fn bulk_delete_with_a_foreign_id_is_404_and_removes_nothing(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;

    create_build(&state, &token, "bulk-a").await;
    let zip = build_zip(&[("mods/a.jar", b"a"), ("mods/b.jar", b"b")]);
    assert_eq!(
        upload_zip(&state, &token, "bulk-a", &zip).await,
        StatusCode::OK
    );

    create_build(&state, &token, "bulk-b").await;
    let other_zip = build_zip(&[("mods/c.jar", b"c")]);
    assert_eq!(
        upload_zip(&state, &token, "bulk-b", &other_zip).await,
        StatusCode::OK
    );

    let mine = artifact_id(&pool, "bulk-a", "mods/a.jar").await;
    let foreign = artifact_id(&pool, "bulk-b", "mods/c.jar").await;
    let before = artifact_paths(&pool, "bulk-a").await;

    // The valid id comes FIRST, so the old loop would have deleted it before hitting the
    // foreign one.
    let status = admin_post_json(
        &state,
        &token,
        "/builds/bulk-a/files/bulk-delete",
        serde_json::json!({ "ids": [mine, foreign] }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a foreign id aborts the batch"
    );

    assert_eq!(
        artifact_paths(&pool, "bulk-a").await,
        before,
        "no row removed by the aborted batch"
    );
    let file = tmp
        .path()
        .join("builds")
        .join("bulk-a")
        .join("files")
        .join("mods")
        .join("a.jar");
    assert!(file.exists(), "no file unlinked by the aborted batch");
}

/// A STALE id (already deleted by someone else) is not a foreign id: it must be skipped
/// so the valid entries in the same batch still go. Two admins with the Files tab open on
/// one build would otherwise lose every deletion to a single stale selection.
#[sqlx::test(migrations = "../../migrations")]
async fn bulk_delete_skips_an_already_deleted_id_and_removes_the_rest(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "bulk-stale";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[("mods/a.jar", b"a"), ("configs/x.cfg", b"x")]);
    assert_eq!(upload_zip(&state, &token, slug, &zip).await, StatusCode::OK);

    let stale = artifact_id(&pool, slug, "mods/a.jar").await;
    let valid = artifact_id(&pool, slug, "configs/x.cfg").await;

    // Admin A deletes it; admin B's list is now stale.
    assert_eq!(
        admin_delete(&state, &token, &format!("/builds/{slug}/files/{stale}")).await,
        StatusCode::OK
    );

    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/bulk-delete"),
        serde_json::json!({ "ids": [stale, valid] }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a vanished id is already-deleted, not an error"
    );
    assert!(
        !artifact_paths(&pool, slug)
            .await
            .contains(&"configs/x.cfg".to_string()),
        "the valid entry in the batch was still removed"
    );
}

/// A well-formed bulk delete still removes every named entry (and a folder's
/// descendants) from both the DB and the disk.
#[sqlx::test(migrations = "../../migrations")]
async fn bulk_delete_removes_every_named_entry(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "bulk-ok";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[
        ("mods/a.jar", b"a"),
        ("configs/x.cfg", b"x"),
        ("configs/y.cfg", b"y"),
    ]);
    assert_eq!(upload_zip(&state, &token, slug, &zip).await, StatusCode::OK);

    let jar = artifact_id(&pool, slug, "mods/a.jar").await;
    let configs = artifact_id(&pool, slug, "configs").await;
    let status = admin_post_json(
        &state,
        &token,
        &format!("/builds/{slug}/files/bulk-delete"),
        serde_json::json!({ "ids": [jar, configs] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        artifact_paths(&pool, slug).await,
        vec!["mods".to_string()],
        "only the surviving parent folder row remains"
    );
    let files = tmp.path().join("builds").join(slug).join("files");
    assert!(!files.join("mods").join("a.jar").exists());
    assert!(!files.join("configs").exists(), "folder subtree unlinked");
}

/// BEQ-14: deleting a build drops the authoritative rows FIRST and the on-disk tree
/// after, so no state can survive pointing at an erased tree.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_build_removes_rows_and_dir(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "goodbye";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[("mods/a.jar", b"a")]);
    assert_eq!(upload_zip(&state, &token, slug, &zip).await, StatusCode::OK);
    let dir = tmp.path().join("builds").join(slug);
    assert!(dir.exists());

    assert_eq!(
        admin_delete(&state, &token, &format!("/builds/{slug}")).await,
        StatusCode::OK
    );

    assert_eq!(bundle_row_count(&pool, slug).await, 0, "bundle row gone");
    assert!(
        artifact_paths(&pool, slug).await.is_empty(),
        "artifacts cascade-deleted"
    );
    assert!(!dir.exists(), "on-disk tree gone");
}

/// BEQ-14: deleting a folder removes the folder row, every descendant row, and the
/// subtree on disk.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_folder_removes_descendants_rows_and_dir(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "folder-delete";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[
        ("mods/deep/a.jar", b"a"),
        ("mods/b.jar", b"b"),
        ("keep/c.cfg", b"c"),
    ]);
    assert_eq!(upload_zip(&state, &token, slug, &zip).await, StatusCode::OK);

    let mods = artifact_id(&pool, slug, "mods").await;
    assert_eq!(
        admin_delete(&state, &token, &format!("/builds/{slug}/files/{mods}")).await,
        StatusCode::OK
    );

    assert_eq!(
        artifact_paths(&pool, slug).await,
        vec!["keep".to_string(), "keep/c.cfg".to_string()],
        "the whole mods subtree is gone, the sibling untouched"
    );
    let files = tmp.path().join("builds").join(slug).join("files");
    assert!(!files.join("mods").exists());
    assert!(files.join("keep").join("c.cfg").exists());
}

/// BE-04 pins the shape `validate` returns now that its whole filesystem half runs on a
/// blocking thread: rows whose file is gone are `missing`, on-disk files no row tracks
/// are `orphaned`, and directories are ignored on both sides.
#[sqlx::test(migrations = "../../migrations")]
async fn validate_reports_missing_and_orphaned(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "validate-build";

    create_build(&state, &token, slug).await;
    let zip = build_zip(&[("mods/a.jar", b"a"), ("mods/b.jar", b"b")]);
    assert_eq!(upload_zip(&state, &token, slug, &zip).await, StatusCode::OK);

    let files = tmp.path().join("builds").join(slug).join("files");
    // Erase a tracked file behind the registry's back, and drop in an untracked one.
    std::fs::remove_file(files.join("mods").join("a.jar")).unwrap();
    std::fs::write(files.join("mods").join("stray.jar"), b"stray").unwrap();

    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/validate"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();

    let missing = body["missing"].as_array().unwrap();
    assert_eq!(missing.len(), 1, "exactly the erased file: {body}");
    assert_eq!(missing[0]["relativePath"], "mods/a.jar");
    assert_eq!(missing[0]["name"], "a.jar");
    assert!(missing[0]["id"].is_string());

    let orphaned = body["orphaned"].as_array().unwrap();
    assert_eq!(orphaned.len(), 1, "exactly the untracked file: {body}");
    assert_eq!(orphaned[0]["relativePath"], "mods/stray.jar");
}

/// A build with no upload at all has no `files/` dir to scan; validate must report two
/// empty lists rather than erroring.
#[sqlx::test(migrations = "../../migrations")]
async fn validate_on_an_empty_build_reports_nothing(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "empty-validate";
    create_build(&state, &token, slug).await;

    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/validate"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(body["missing"].as_array().unwrap().is_empty());
    assert!(body["orphaned"].as_array().unwrap().is_empty());
}

/// CB-11: the staged upload file is owned by a `TempUpload` guard rather than by seven
/// hand-written `remove_file` arms. Every failure path must leave no `.zip.tmp` behind —
/// a leaked one holds up to `MAX_UPLOAD_BYTES` (10 GiB) of the storage volume.
#[sqlx::test(migrations = "../../migrations")]
async fn failed_uploads_leave_no_staged_temp_file(pool: PgPool) {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(pool.clone(), tmp.path());
    let token = seed_admin_token(&pool).await;
    let slug = "leaky";
    create_build(&state, &token, slug).await;

    let build_dir = tmp.path().join("builds").join(slug);
    let staged = || -> Vec<std::path::PathBuf> {
        std::fs::read_dir(&build_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.to_string_lossy().ends_with(".zip.tmp"))
                    .collect()
            })
            .unwrap_or_default()
    };

    // (1) A multipart body with no `archive` field: rejected after the temp path exists.
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/upload"))
                .header(
                    "content-type",
                    "multipart/form-data; boundary=----boundaryBUNDLE",
                )
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    "------boundaryBUNDLE\r\n\
                     Content-Disposition: form-data; name=\"notarchive\"\r\n\r\n\
                     x\r\n------boundaryBUNDLE--\r\n",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "no archive field");
    assert!(staged().is_empty(), "staged temp leaked: {:?}", staged());

    // (2) An `archive` field that is not a ZIP: ingest fails, status flips to failed.
    let status = upload_zip(&state, &token, slug, b"this is not a zip at all").await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "{status}"
    );
    assert!(staged().is_empty(), "staged temp leaked: {:?}", staged());

    // (3) A single-file upload with no `file` field.
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/files"))
                .header(
                    "content-type",
                    "multipart/form-data; boundary=----boundaryBUNDLE",
                )
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    "------boundaryBUNDLE\r\n\
                     Content-Disposition: form-data; name=\"targetPath\"\r\n\r\n\
                     mods/x.jar\r\n------boundaryBUNDLE--\r\n",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "no file field");
    assert!(staged().is_empty(), "staged temp leaked: {:?}", staged());
}
