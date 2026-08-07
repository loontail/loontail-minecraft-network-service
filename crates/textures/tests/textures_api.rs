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
use loontail_core::auth::issue_session;
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
    // why: texture URLs are built from the Yggdrasil public URL, so the assertions below
    // would follow whatever `YGGDRASIL_PUBLIC_URL` the ambient env/.env happens to set.
    // Pin it here instead of demanding the runner export it.
    config.yggdrasil.public_url = "/api/yggdrasil".to_string();

    let state = AppState::new(pool, config);
    let router = Router::new()
        .nest("/textures", loontail_textures::routes())
        .with_state(state.clone());
    (router, root, state)
}

/// A standalone config when `DATABASE_URL` is unset in the test process; the pool
/// is injected by `#[sqlx::test]`, so the DB URL here is never used.
fn test_config() -> Config {
    // why: only a placeholder when the var is absent — clobbering a real DATABASE_URL
    // trips sqlx::test's "DATABASE_URL changed at runtime" assertion mid-run.
    if std::env::var_os("DATABASE_URL").is_none() {
        std::env::set_var("DATABASE_URL", "postgres://unused");
    }
    Config::from_env().unwrap()
}

/// Seed a confirmed user and issue a unified session token; return
/// (profile_uuid, token). Textures now authenticate with the launcher's SESSION
/// bearer (not the Yggdrasil game token).
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
    let session = issue_session(pool, user.id, Duration::from_secs(900))
        .await
        .unwrap();
    (user.profile_uuid.unwrap(), session.token)
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
        sqlx::query_as("SELECT file_url, file_path, variant FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'")
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
    assert!(json["cape"].is_null());
}

#[sqlx::test(migrations = "../../migrations")]
async fn upload_skin_64x32_is_accepted(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "legacy").await;

    let status = put_texture(&router, "skin", &token, &png(64, 32), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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

    let variant: String = sqlx::query_scalar(
        "SELECT variant FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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
async fn reject_oversized_upload(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "whale").await;

    // A valid PNG header followed by padding that pushes the `file` field past the
    // 256 KiB cap. The streaming reader (or the route body limit) rejects it before
    // any row or file is created, so the upload never lands.
    let mut huge = png(64, 64);
    huge.resize(loontail_textures::MAX_UPLOAD_BYTES + 1, 0u8);
    let status = put_texture(&router, "skin", &token, &huge, None).await;
    assert!(
        status.is_client_error(),
        "oversized upload must be rejected, got {status}"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
    .bind(&profile_uuid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "oversized upload must not create a row");
}

#[sqlx::test(migrations = "../../migrations")]
async fn replace_skin_unlinks_old_file_and_writes_new(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "replacer").await;

    put_texture(&router, "skin", &token, &png(64, 64), None).await;
    let old_path: String = sqlx::query_scalar(
        "SELECT file_path FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
    .bind(&profile_uuid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(std::path::Path::new(&old_path).exists());

    // Re-upload — a fresh revision is written and the old file is unlinked.
    put_texture(&router, "skin", &token, &png(64, 32), None).await;
    let new_path: String = sqlx::query_scalar(
        "SELECT file_path FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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
    let file_path: String = sqlx::query_scalar(
        "SELECT file_path FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
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
    assert!(json["skin"].is_null());
}

#[sqlx::test(migrations = "../../migrations")]
async fn lookup_returns_both_kinds_for_one_user(pool: PgPool) {
    // CB-1: skins and capes share one `user_textures` table keyed on (user_id, kind),
    // so one query serves both. A user carrying both must get both back, with the
    // skin's variant and no variant leaking onto the cape.
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, token) = seed_user_with_token(&pool, "dressed").await;

    assert_eq!(
        put_texture(&router, "skin", &token, &png(64, 64), Some("SLIM")).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_texture(&router, "cape", &token, &png(64, 32), None).await,
        StatusCode::NO_CONTENT
    );

    let (status, json) = get_json(&router, &format!("/textures/{profile_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["skin"]["url"],
        format!("/textures/{profile_uuid}/skin")
    );
    assert_eq!(json["skin"]["variant"], "SLIM");
    assert_eq!(
        json["cape"]["url"],
        format!("/textures/{profile_uuid}/cape")
    );
    assert!(json["cape"]["variant"].is_null());

    // The kind discriminator keeps the two rows distinct rather than upserting over
    // each other, and only the skin row carries a variant.
    let kinds: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT kind, variant FROM user_textures WHERE profile_uuid = $1 ORDER BY kind",
    )
    .bind(&profile_uuid)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        kinds,
        vec![
            ("cape".to_string(), None),
            ("skin".to_string(), Some("SLIM".to_string())),
        ]
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn lookup_absent_profile_returns_null_skin_and_cape(pool: PgPool) {
    let (router, _root, _s) = app(pool.clone());
    let (profile_uuid, _token) = seed_user_with_token(&pool, "naked").await;

    let (status, json) = get_json(&router, &format!("/textures/{profile_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    // Both keys present and null (the launcher's TexturesLookupResponseSchema
    // requires the keys; an empty `{}` fails its parse).
    assert_eq!(json, serde_json::json!({ "skin": null, "cape": null }));
}

#[sqlx::test(migrations = "../../migrations")]
async fn skin_survives_profile_uuid_reconcile(pool: PgPool) {
    // BUG-2: a credential-only user uploads a skin (row keyed by user_id with a
    // denormalized profile_uuid = the random one). Later the user's minecraft_uuid
    // becomes known and identity reconciliation rewrites users.profile_uuid. The skin
    // must still be served under the NEW profile_uuid (the row's stale denormalized
    // profile_uuid must not orphan it), and the URL the signed GameProfile embeds
    // (`/textures/{newUuid}/skin`) must resolve.
    use loontail_core::identity::{assign_or_reconcile_profile_uuid, load_user};

    let (textures, _root, _state) = app(pool.clone());

    // register → launcher login → upload skin, all under the random profile_uuid.
    let user = admin_create_user(
        &pool,
        AdminCreateUser {
            username: "rebound".into(),
            email: "rebound@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    let user_id = user.id;
    let old_uuid = user.profile_uuid.clone().unwrap();
    let token = issue_session(&pool, user_id, Duration::from_secs(900))
        .await
        .unwrap()
        .token;

    assert_eq!(
        put_texture(&textures, "skin", &token, &png(64, 64), Some("SLIM")).await,
        StatusCode::NO_CONTENT
    );

    // The skin serves under the original profile_uuid.
    let (status, json) = get_json(&textures, &format!("/textures/{old_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["skin"]["url"], format!("/textures/{old_uuid}/skin"));

    // The in-game bootstrap binds a minecraft_uuid to this row; the next login
    // reconciles profile_uuid := undash(minecraft_uuid). Simulate that here.
    let minecraft_uuid = "11111111-2222-3333-4444-555555555555";
    let new_uuid = "11111111222233334444555555555555";
    sqlx::query("UPDATE users SET minecraft_uuid = $2 WHERE id = $1")
        .bind(user_id)
        .bind(minecraft_uuid)
        .execute(&pool)
        .await
        .unwrap();
    let bound = load_user(&pool, user_id).await.unwrap();
    let reconciled = assign_or_reconcile_profile_uuid(&pool, &bound)
        .await
        .unwrap();
    assert_eq!(reconciled.profile_uuid.as_deref(), Some(new_uuid));
    assert_ne!(old_uuid, new_uuid);

    // The texture row still carries the OLD denormalized profile_uuid...
    let stale: String = sqlx::query_scalar(
        "SELECT profile_uuid FROM user_textures WHERE user_id = $1 AND kind = 'skin'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        stale, old_uuid,
        "row's denormalized profile_uuid went stale"
    );

    // ...yet the skin is served under the NEW profile_uuid (joined via users.id).
    let (status, json) = get_json(&textures, &format!("/textures/{new_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["skin"]["url"], format!("/textures/{new_uuid}/skin"));
    assert_eq!(json["skin"]["variant"], "SLIM");

    // The PNG byte route the signed GameProfile texture URL points at resolves.
    let png_req = Request::builder()
        .method("GET")
        .uri(format!("/textures/{new_uuid}/skin"))
        .body(Body::empty())
        .unwrap();
    let png_resp = textures.clone().oneshot(png_req).await.unwrap();
    assert_eq!(png_resp.status(), StatusCode::OK);
    assert_eq!(
        png_resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );

    // The OLD uuid no longer resolves (no user carries it now).
    let (status, json) = get_json(&textures, &format!("/textures/{old_uuid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, serde_json::json!({ "skin": null, "cape": null }));
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
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// --- Admin moderation surface (`/admin/textures/*`) --------------------------

/// The admin router (gated by `AdminUser`), sharing the test's pool + storage.
fn admin_app(state: &AppState) -> Router {
    Router::new()
        .nest("/admin/textures", loontail_textures::admin_routes())
        .with_state(state.clone())
}

/// Seed an admin and return its Bearer token (the `AdminUser` extractor accepts a
/// session bearer and is CSRF-exempt for it).
async fn seed_admin_token(pool: &PgPool, name: &str) -> String {
    let user = admin_create_user(
        pool,
        AdminCreateUser {
            username: name.into(),
            email: format!("{name}@example.com"),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: true,
        },
    )
    .await
    .unwrap();
    issue_session(pool, user.id, Duration::from_secs(900))
        .await
        .unwrap()
        .token
}

async fn admin_request(
    router: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let resp = router
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
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
async fn admin_lists_and_deletes_a_skin(pool: PgPool) {
    let (textures, _root, state) = app(pool.clone());
    let admin = admin_app(&state);
    let admin_token = seed_admin_token(&pool, "boss").await;
    let (profile_uuid, user_token) = seed_user_with_token(&pool, "painter").await;

    // The painter uploads a skin through the normal (player) route.
    assert_eq!(
        put_texture(&textures, "skin", &user_token, &png(64, 64), Some("SLIM")).await,
        StatusCode::NO_CONTENT
    );

    // Admin listing sees exactly that one row with the right fields.
    let (status, json) =
        admin_request(&admin, "GET", "/admin/textures/skins", Some(&admin_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["meta"]["total"], 1);
    assert_eq!(json["data"][0]["username"], "painter");
    assert_eq!(json["data"][0]["profileUuid"], profile_uuid);
    assert_eq!(json["data"][0]["variant"], "SLIM");

    let user_id = json["data"][0]["userId"].as_str().unwrap().to_string();
    let file_path: String = sqlx::query_scalar(
        "SELECT file_path FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
    .bind(&profile_uuid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(std::path::Path::new(&file_path).exists());

    // Admin delete removes the row and unlinks the file.
    let (status, json) = admin_request(
        &admin,
        "DELETE",
        &format!("/admin/textures/skins/{user_id}"),
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["deleted"], true);
    assert!(!std::path::Path::new(&file_path).exists());

    let (_status, json) =
        admin_request(&admin, "GET", "/admin/textures/skins", Some(&admin_token)).await;
    assert_eq!(json["meta"]["total"], 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_search_filters_by_username(pool: PgPool) {
    let (textures, _root, state) = app(pool.clone());
    let admin = admin_app(&state);
    let admin_token = seed_admin_token(&pool, "boss").await;
    let (_p1, t1) = seed_user_with_token(&pool, "alice").await;
    let (_p2, t2) = seed_user_with_token(&pool, "bob").await;
    put_texture(&textures, "skin", &t1, &png(64, 64), None).await;
    put_texture(&textures, "skin", &t2, &png(64, 64), None).await;

    let (status, json) = admin_request(
        &admin,
        "GET",
        "/admin/textures/skins?q=alic",
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["meta"]["total"], 1);
    assert_eq!(json["data"][0]["username"], "alice");
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_orphans_scan_and_purge(pool: PgPool) {
    let (textures, _root, state) = app(pool.clone());
    let admin = admin_app(&state);
    let admin_token = seed_admin_token(&pool, "boss").await;
    let (profile_uuid, user_token) = seed_user_with_token(&pool, "ghost").await;
    put_texture(&textures, "skin", &user_token, &png(64, 64), None).await;

    // Simulate DB/disk drift: delete the file but keep the row.
    let file_path: String = sqlx::query_scalar(
        "SELECT file_path FROM user_textures WHERE profile_uuid = $1 AND kind = 'skin'",
    )
    .bind(&profile_uuid)
    .fetch_one(&pool)
    .await
    .unwrap();
    std::fs::remove_file(&file_path).unwrap();

    // The orphan scan reports the row whose file is gone.
    let (status, json) =
        admin_request(&admin, "GET", "/admin/textures/orphans", Some(&admin_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["skins"].as_array().unwrap().len(), 1);
    assert_eq!(json["skins"][0]["profileUuid"], profile_uuid);

    // Purge removes it.
    let (status, json) = admin_request(
        &admin,
        "POST",
        "/admin/textures/purge-missing",
        Some(&admin_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["purgedSkins"], 1);

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM user_textures")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_routes_require_admin_auth(pool: PgPool) {
    let (_textures, _root, state) = app(pool.clone());
    let admin = admin_app(&state);

    // No token at all.
    let (status, _json) = admin_request(&admin, "GET", "/admin/textures/skins", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A valid NON-admin session is rejected (requires is_admin).
    let (_profile, user_token) = seed_user_with_token(&pool, "peon").await;
    let (status, _json) =
        admin_request(&admin, "GET", "/admin/textures/skins", Some(&user_token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
