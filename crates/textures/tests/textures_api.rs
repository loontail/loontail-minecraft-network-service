//! Integration tests for the textures domain: drive the `Router<AppState>` mounted
//! at `/textures` via `tower::ServiceExt::oneshot` against an isolated Postgres per
//! test (`#[sqlx::test]`). Each test gets its own temp storage root so on-disk
//! assertions don't collide.

use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use loontail_core::auth::yggdrasil::issue_yggdrasil_tokens;
use loontail_core::config::Config;
use loontail_core::identity::{admin_create_user, AdminCreateUser};
use loontail_core::AppState;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// A minimal valid PNG: the 8-byte signature + a 13-byte IHDR carrying the given
/// width/height. The validator reads only this header, so it is a sufficient
/// fixture and the same bytes round-trip through the PNG read endpoint.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&PNG_SIGNATURE);
    buf.extend_from_slice(&13u32.to_be_bytes());
    buf.extend_from_slice(b"IHDR");
    buf.extend_from_slice(&width.to_be_bytes());
    buf.extend_from_slice(&height.to_be_bytes());
    buf
}

/// Build a per-test storage root under the OS temp dir and an `AppState`/`Router`
/// wired to it. Returns the storage root for on-disk assertions.
fn app(pool: PgPool) -> (Router, PathBuf, AppState) {
    let mut config = Config::from_env().unwrap_or_else(|_| test_config());
    let root = std::env::temp_dir().join(format!("loontail-textures-test-{}", Uuid::new_v4()));
    config.textures.storage_root = root.to_string_lossy().to_string();

    let state = AppState::new(pool, config);
    let router = Router::new()
        .nest("/textures", loontail_textures::routes())
        .with_state(state.clone());
    (router, root, state)
}

/// A standalone config when `DATABASE_URL` is unset in the test process; the pool
/// is injected by `#[sqlx::test]`, so the DB URL here is never used.
fn test_config() -> Config {
    std::env::set_var("DATABASE_URL", "postgres://unused");
    Config::from_env().unwrap()
}

/// Seed a confirmed Yggdrasil user and issue a token; return (profile_uuid, token).
async fn seed_user_with_token(pool: &PgPool, name: &str) -> (String, String) {
    let user = admin_create_user(
        pool,
        AdminCreateUser {
            username: name.into(),
            email: format!("{name}@example.com"),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    let tokens = issue_yggdrasil_tokens(pool, user.id, None, Duration::from_secs(900), 10)
        .await
        .unwrap();
    (user.profile_uuid.unwrap(), tokens.access_token)
}

/// Build a `multipart/form-data` body with a `file` part and optional `variant`.
fn multipart_body(file: &[u8], variant: Option<&str>) -> (String, Vec<u8>) {
    let boundary = "----loontailtestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"t.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n");
    if let Some(v) = variant {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"variant\"\r\n\r\n");
        body.extend_from_slice(v.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn put_texture(
    router: &Router,
    kind: &str,
    token: &str,
    file: &[u8],
    variant: Option<&str>,
) -> StatusCode {
    let (content_type, body) = multipart_body(file, variant);
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/textures/{kind}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    router.clone().oneshot(req).await.unwrap().status()
}

async fn get_json(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_skin_64x64_creates_row_file_and_lookup(pool: PgPool) {
    let (router, root, _state) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "skinner").await;

    let status = put_texture(&router, "skin", &token, &png(64, 64), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Row created with the right url + variant default.
    let (file_url, file_path, variant): (String, String, String) =
        sqlx::query_as("SELECT file_url, file_path, variant FROM skins WHERE profile_uuid = $1")
            .bind(&profile_uuid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(file_url, format!("/textures/{profile_uuid}/skin"));
    assert_eq!(variant, "CLASSIC");

    // File is on disk under the storage root.
    assert!(std::path::Path::new(&file_path).exists());
    assert!(file_path.starts_with(&root.to_string_lossy().to_string()));

    // Lookup returns the skin url + variant, no cape.
    let (status, json) = get_json(&router, &format!("/textures/{profile_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["skin"]["url"],
        format!("/textures/{profile_uuid}/skin")
    );
    assert_eq!(json["skin"]["variant"], "CLASSIC");
    assert!(json.get("cape").is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_skin_64x32_is_accepted(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "legacy").await;

    let status = put_texture(&router, "skin", &token, &png(64, 32), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM skins WHERE profile_uuid = $1")
        .bind(&profile_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_skin_slim_variant_is_stored_and_returned(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "slimjim").await;

    let status = put_texture(&router, "skin", &token, &png(64, 64), Some("SLIM")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let variant: String = sqlx::query_scalar("SELECT variant FROM skins WHERE profile_uuid = $1")
        .bind(&profile_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(variant, "SLIM");

    let (_status, json) = get_json(&router, &format!("/textures/{profile_uuid}")).await;
    assert_eq!(json["skin"]["variant"], "SLIM");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reject_invalid_png_bad_dimensions(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "baddims").await;

    // 32x32 is not a valid skin size.
    let status = put_texture(&router, "skin", &token, &png(32, 32), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM skins WHERE profile_uuid = $1")
        .bind(&profile_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "rejected upload must not create a row");
}

#[sqlx::test(migrations = "../../migrations")]
async fn reject_invalid_png_bad_signature(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (_profile_uuid, token) = seed_user_with_token(&pool, "badsig").await;

    let mut bad = png(64, 64);
    bad[0] = 0x00; // corrupt the signature
    let status = put_texture(&router, "skin", &token, &bad, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "../../migrations")]
async fn replace_skin_unlinks_old_file_and_writes_new(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "replacer").await;

    put_texture(&router, "skin", &token, &png(64, 64), None).await;
    let old_path: String =
        sqlx::query_scalar("SELECT file_path FROM skins WHERE profile_uuid = $1")
            .bind(&profile_uuid)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(std::path::Path::new(&old_path).exists());

    // Re-upload — a fresh revision is written and the old file is unlinked.
    put_texture(&router, "skin", &token, &png(64, 32), None).await;
    let new_path: String =
        sqlx::query_scalar("SELECT file_path FROM skins WHERE profile_uuid = $1")
            .bind(&profile_uuid)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_ne!(old_path, new_path, "new revision has a distinct path");
    assert!(
        !std::path::Path::new(&old_path).exists(),
        "old file unlinked"
    );
    assert!(std::path::Path::new(&new_path).exists(), "new file present");

    // Exactly one row remains (upsert, not insert).
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM skins WHERE profile_uuid = $1")
        .bind(&profile_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_skin_removes_row_and_png_then_404(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "deleter").await;

    put_texture(&router, "skin", &token, &png(64, 64), None).await;
    let file_path: String =
        sqlx::query_scalar("SELECT file_path FROM skins WHERE profile_uuid = $1")
            .bind(&profile_uuid)
            .fetch_one(&pool)
            .await
            .unwrap();

    // PNG endpoint serves the bytes while present.
    let png_req = Request::builder()
        .method("GET")
        .uri(format!("/textures/{profile_uuid}/skin"))
        .body(Body::empty())
        .unwrap();
    let png_resp = router.clone().oneshot(png_req).await.unwrap();
    assert_eq!(png_resp.status(), StatusCode::OK);
    assert_eq!(
        png_resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );

    // Delete.
    let del = Request::builder()
        .method("DELETE")
        .uri("/textures/skin")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let del_status = router.clone().oneshot(del).await.unwrap().status();
    assert_eq!(del_status, StatusCode::NO_CONTENT);

    // Row gone, file gone.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM skins WHERE profile_uuid = $1")
        .bind(&profile_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(!std::path::Path::new(&file_path).exists());

    // PNG endpoint now 404s.
    let (status, _json) = get_json(&router, &format!("/textures/{profile_uuid}/skin")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_cape_then_lookup_includes_cape(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "caped").await;

    // A 64x64 cape is rejected (capes are 64x32 only).
    let bad = put_texture(&router, "cape", &token, &png(64, 64), None).await;
    assert_eq!(bad, StatusCode::BAD_REQUEST);

    let ok = put_texture(&router, "cape", &token, &png(64, 32), None).await;
    assert_eq!(ok, StatusCode::NO_CONTENT);

    let (_status, json) = get_json(&router, &format!("/textures/{profile_uuid}")).await;
    assert_eq!(
        json["cape"]["url"],
        format!("/textures/{profile_uuid}/cape")
    );
    assert!(json.get("skin").is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn lookup_absent_profile_returns_empty_object(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, _token) = seed_user_with_token(&pool, "naked").await;

    let (status, json) = get_json(&router, &format!("/textures/{profile_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("skin").is_none());
    assert!(json.get("cape").is_none());
    assert_eq!(json, serde_json::json!({}));
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_requires_auth(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());

    let (content_type, body) = multipart_body(&png(64, 64), None);
    let req = Request::builder()
        .method("PUT")
        .uri("/textures/skin")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    let status = router.clone().oneshot(req).await.unwrap().status();
    assert_eq!(status, StatusCode::FORBIDDEN);
}
