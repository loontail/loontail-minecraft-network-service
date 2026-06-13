//! Integration tests for the bundle registry: ZIP ingest, on-disk layout,
//! artifact rows + sha256, manifest byte-shape, zip-slip rejection, and the
//! public manifest contract (served verbatim from artifacts.json).

use std::io::Write;
use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use loontail_core::auth::admin::issue_admin_session;
use loontail_core::config::Config;
use loontail_core::identity::{admin_create_user, AdminCreateUser};
use loontail_core::AppState;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::util::ServiceExt;

const STORAGE_PUBLIC_PREFIX: &str = "/bundle-registry";

/// Build an `AppState` whose bundle storage root points at a fresh tempdir.
fn test_state(pool: PgPool, storage_root: &Path) -> AppState {
    std::env::set_var("DATABASE_URL", "postgres://unused");
    let mut config = Config::from_env().expect("config");
    config.bundles.storage_root = storage_root.to_string_lossy().into_owned();
    config.bundles.public_url = STORAGE_PUBLIC_PREFIX.to_string();
    AppState::new(pool, config)
}

async fn seed_admin_cookie(pool: &PgPool) -> String {
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
    let session = issue_admin_session(pool, admin.id, std::time::Duration::from_secs(900))
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

async fn create_build(state: &AppState, cookie: &str, slug: &str) {
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let body = serde_json::json!({ "name": "Test Bundle", "slug": slug }).to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/builds")
                .header("content-type", "application/json")
                .header("cookie", format!("loontail_admin={cookie}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "create build");
}

async fn upload_zip(state: &AppState, cookie: &str, slug: &str, zip: &[u8]) -> StatusCode {
    let app = loontail_bundles::admin_routes().with_state(state.clone());
    let (content_type, body) = multipart_zip_body(zip);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/builds/{slug}/upload"))
                .header("content-type", content_type)
                .header("cookie", format!("loontail_admin={cookie}"))
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
    let cookie = seed_admin_cookie(&pool).await;
    let slug = "my-build";

    create_build(&state, &cookie, slug).await;
    let status = upload_zip(&state, &cookie, slug, &build_test_zip()).await;
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
    let cookie = seed_admin_cookie(&pool).await;
    let slug = "contract-build";

    create_build(&state, &cookie, slug).await;
    upload_zip(&state, &cookie, slug, &build_test_zip()).await;

    // The bytes on disk are the source of truth.
    let manifest_path = tmp.path().join("builds").join(slug).join("artifacts.json");
    let on_disk = std::fs::read(&manifest_path).unwrap();

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
    let cookie = seed_admin_cookie(&pool).await;
    let slug = "static-build";

    create_build(&state, &cookie, slug).await;
    upload_zip(&state, &cookie, slug, &build_test_zip()).await;

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
    let cookie = seed_admin_cookie(&pool).await;
    let slug = "evil-build";

    create_build(&state, &cookie, slug).await;

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

    let status = upload_zip(&state, &cookie, slug, &zip_bytes).await;
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
    let cookie = seed_admin_cookie(&pool).await;
    let slug = "once-build";

    create_build(&state, &cookie, slug).await;
    upload_zip(&state, &cookie, slug, &build_test_zip()).await;

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
                .header("cookie", format!("loontail_admin={cookie}"))
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
