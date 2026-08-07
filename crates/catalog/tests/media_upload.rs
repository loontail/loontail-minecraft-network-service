//! Wave A (P0-2/P0-3, P2-1): catalog-media upload behavior and on-disk cleanup.
//!
//! - A 9 MiB upload now succeeds (the cap rose from 8 MiB to 32 MiB).
//! - A 33 MiB body returns the friendly JSON 400 ("image is too large"), not an
//!   opaque non-JSON 413 from axum's body-limit layer (the route limit sits a MiB
//!   above the in-handler cap so the handler always wins).
//! - A GIF magic-byte file is accepted (201).
//! - Deleting a client removes its `{storage_root}/{client_hex}/` media dir from
//!   disk, not just the DB rows.

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
/// real on-disk media tree being created and then deleted.
struct TempRoot {
    path: std::path::PathBuf,
}

impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("llapi-media-test-{}", Uuid::new_v4()));
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

/// Point both the catalog media storage root and the bundles storage root at temp
/// dirs so the upload writes real bytes and `delete_client`'s on-disk cleanup runs.
fn state_with_roots(pool: PgPool, catalog_root: &TempRoot, bundles_root: &TempRoot) -> AppState {
    let mut config = Config::from_env().expect("config from env");
    config.catalog.storage_root = catalog_root.as_str().to_string();
    config.bundles.storage_root = bundles_root.as_str().to_string();
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

async fn create_client(state: AppState, token: &str, slug: &str) -> Uuid {
    let body = json!({
        "slug": slug,
        "available": true,
        "locales": [{ "locale": "en", "title": slug }],
    });
    let app = loontail_catalog::admin_routes().with_state(state);
    let req = Request::builder()
        .method("POST")
        .uri("/clients")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    Uuid::parse_str(json.get("id").and_then(Value::as_str).unwrap()).unwrap()
}

/// Build a `multipart/form-data` body with a `file` part and a `role` text part.
fn multipart_body(file: &[u8], role: &str) -> (String, Vec<u8>) {
    let boundary = "----loontailmediaboundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"m.bin\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"role\"\r\n\r\n");
    body.extend_from_slice(role.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// A minimal valid PNG header followed by padding to reach `total` bytes. The
/// upload only sniffs the magic bytes + reads the IHDR for PNG, so padding is fine.
fn png_of_size(total: usize) -> Vec<u8> {
    // PNG signature + "IHDR" at offset 12 + 1x1 dimensions, then zero padding.
    let mut v = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, // IHDR length
        b'I', b'H', b'D', b'R', // IHDR
        0x00, 0x00, 0x00, 0x01, // width = 1
        0x00, 0x00, 0x00, 0x01, // height = 1
    ];
    v.resize(total.max(v.len()), 0);
    v
}

/// A GIF89a header + padding. Magic `GIF8` is what `sniff_image` keys on.
fn gif_bytes() -> Vec<u8> {
    let mut v = b"GIF89a".to_vec();
    v.extend_from_slice(&[0u8; 32]);
    v
}

async fn upload(
    state: AppState,
    token: &str,
    client_id: Uuid,
    file: &[u8],
    role: &str,
) -> (StatusCode, Vec<u8>, Option<String>) {
    let (content_type, body) = multipart_body(file, role);
    let app = loontail_catalog::admin_routes().with_state(state);
    let req = Request::builder()
        .method("POST")
        .uri(format!("/clients/{client_id}/media/upload"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let ct = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, bytes, ct)
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_9mib_png_succeeds(pool: PgPool) {
    let catalog_root = TempRoot::new();
    let bundles_root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let client_id = create_client(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        "nine-mib",
    )
    .await;

    let file = png_of_size(9 * 1024 * 1024);
    let (status, body, _) = upload(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        client_id,
        &file,
        "poster",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "9 MiB upload should succeed under the 32 MiB cap; body: {}",
        String::from_utf8_lossy(&body)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn oversized_upload_returns_json_400_not_opaque_413(pool: PgPool) {
    let catalog_root = TempRoot::new();
    let bundles_root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let client_id = create_client(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        "too-big",
    )
    .await;

    // Just over the 32 MiB in-handler cap, but the whole multipart body stays under
    // the route's body limit (cap + 1 MiB) so the handler's friendly JSON 400 wins
    // over axum's opaque 413/parse error.
    let file = png_of_size(32 * 1024 * 1024 + 256 * 1024);
    let (status, body, content_type) = upload(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        client_id,
        &file,
        "poster",
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "oversized body must hit the in-handler JSON 400, not axum's opaque 413"
    );
    assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        content_type
            .as_deref()
            .unwrap_or("")
            .contains("application/json"),
        "rejection is JSON, got content-type {content_type:?}"
    );
    let json: Value = serde_json::from_slice(&body).expect("rejection body is JSON");
    let msg = serde_json::to_string(&json).unwrap();
    assert!(
        msg.contains("too large"),
        "message mentions the size limit, got {msg}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_gif_succeeds(pool: PgPool) {
    let catalog_root = TempRoot::new();
    let bundles_root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let client_id = create_client(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        "gif-build",
    )
    .await;

    let (status, body, _) = upload(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        client_id,
        &gif_bytes(),
        "poster",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "GIF magic bytes should be accepted; body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: Value = serde_json::from_slice(&body).unwrap();
    let url = json.get("url").and_then(Value::as_str).unwrap();
    assert!(url.ends_with(".gif"), "stored as .gif, got {url}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_client_removes_media_dir(pool: PgPool) {
    let catalog_root = TempRoot::new();
    let bundles_root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let client_id = create_client(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        "with-media",
    )
    .await;

    // Upload a real file so the client's on-disk media dir exists.
    let (status, _, _) = upload(
        state_with_roots(pool.clone(), &catalog_root, &bundles_root),
        &token,
        client_id,
        &png_of_size(64),
        "poster",
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let media_dir =
        std::path::Path::new(catalog_root.as_str()).join(client_id.simple().to_string());
    assert!(media_dir.exists(), "media dir exists after upload");

    // Delete the build.
    let app = loontail_catalog::admin_routes().with_state(state_with_roots(
        pool.clone(),
        &catalog_root,
        &bundles_root,
    ));
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/clients/{client_id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    assert!(
        !media_dir.exists(),
        "client media dir removed from disk after delete"
    );
}

/// CB-20: `png_dimensions` delegates to the workspace's tested PNG header parser
/// instead of re-reading the IHDR offsets by hand. The inlined copy validated neither
/// the 8-byte signature nor the IHDR length, so a hostile GIF whose bytes 12..16
/// happened to spell "IHDR" had its "dimensions" recorded as real pixel sizes.
#[sqlx::test(migrations = "../../migrations")]
async fn only_a_real_png_records_dimensions(pool: PgPool) {
    let catalog_root = TempRoot::new();
    let bundles_root = TempRoot::new();
    let token = seed_admin_token(&pool).await;
    let state = state_with_roots(pool.clone(), &catalog_root, &bundles_root);
    let client_id = create_client(state.clone(), &token, "dims").await;

    let (status, _, _) = upload(state.clone(), &token, client_id, &png_of_size(64), "poster").await;
    assert_eq!(status, StatusCode::CREATED);

    // A GIF that spells IHDR where a PNG's IHDR would be, with 4096x4096 in the width
    // and height slots. `sniff_image` accepts it as a GIF; it must NOT get dimensions.
    let mut liar = b"GIF89a".to_vec();
    liar.resize(12, 0);
    liar.extend_from_slice(b"IHDR");
    liar.extend_from_slice(&4096u32.to_be_bytes());
    liar.extend_from_slice(&4096u32.to_be_bytes());
    liar.resize(64, 0);
    let (status, _, _) = upload(state, &token, client_id, &liar, "background").await;
    assert_eq!(status, StatusCode::CREATED);

    let dims: Vec<(String, Option<i32>, Option<i32>)> =
        sqlx::query_as("SELECT role, width, height FROM catalog_media ORDER BY role")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        dims,
        vec![
            ("background".to_string(), None, None),
            ("poster".to_string(), Some(1), Some(1)),
        ]
    );
}
