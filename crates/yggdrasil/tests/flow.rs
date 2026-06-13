//! End-to-end Yggdrasil integration tests over an isolated Postgres per test,
//! driving the real router via `tower::ServiceExt::oneshot` (no socket).
//!
//! Covers: authenticate → validate → refresh → invalidate, and a join → hasJoined
//! flow that produces an RSA-SHA1-signed `GameProfile`.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use loontail_core::config::{
    AdminConfig, BundlesConfig, Config, RateLimitConfig, RequestLogConfig, TexturesConfig,
    YggdrasilConfig,
};
use loontail_core::identity::{admin_create_user, AdminCreateUser};
use loontail_core::AppState;
use loontail_yggdrasil::{init_crypto, routes, GameProfile};
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PUBLIC_URL: &str = "/api/yggdrasil";

fn test_config() -> Config {
    Config {
        http_host: "127.0.0.1".into(),
        http_port: 0,
        database_url: String::new(),
        cors_allowed_origins: vec![],
        session_ttl: Duration::from_secs(86_400),
        heartbeat_timeout: Duration::from_secs(60),
        max_players_per_world: 5,
        join_request_ttl: Duration::from_secs(60),
        join_ticket_ttl: Duration::from_secs(60),
        invite_ttl: Duration::from_secs(600),
        search_min_query_length: 2,
        search_max_results: 20,
        rate_limit: RateLimitConfig {
            max_attempts: 10,
            window: Duration::from_secs(60),
        },
        request_log: RequestLogConfig { retention_days: 7 },
        yggdrasil: YggdrasilConfig {
            public_url: PUBLIC_URL.into(),
            key_path: "tests/test-key.pem".into(),
            token_ttl: Duration::from_secs(900),
            max_tokens_per_user: 10,
            skin_domains: vec![".loontail.com".into(), "localhost".into()],
        },
        textures: TexturesConfig {
            storage_root: "data/textures".into(),
        },
        bundles: BundlesConfig {
            storage_root: "data/bundle-registry".into(),
            public_url: "/bundle-registry".into(),
        },
        admin: AdminConfig {
            session_ttl: Duration::from_secs(604_800),
            cookie_name: "loontail_admin".into(),
            bootstrap_username: "admin".into(),
            bootstrap_password: None,
        },
    }
}

fn app(pool: PgPool) -> axum::Router {
    let config = test_config();
    init_crypto(&config).expect("init crypto from test key");
    let state = AppState::new(pool, config);
    routes().with_state(state)
}

async fn seed_user(pool: &PgPool, name: &str, password: &str) -> (Uuid, String) {
    let user = admin_create_user(
        pool,
        AdminCreateUser {
            username: name.into(),
            email: format!("{name}@example.com"),
            password: password.into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    (user.id, user.profile_uuid.unwrap())
}

async fn post_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

async fn get_status_body(app: &axum::Router, path: &str) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, bytes.to_vec())
}

#[sqlx::test(migrations = "../../migrations")]
async fn authenticate_validate_refresh_invalidate(pool: PgPool) {
    let (_uid, profile_uuid) = seed_user(&pool, "alice", "hunter2").await;
    let app = app(pool);

    // authenticate
    let (status, body) = post_json(
        &app,
        "/authserver/authenticate",
        json!({ "username": "alice", "password": "hunter2", "requestUser": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "authenticate ok: {body}");
    let access = body["accessToken"].as_str().unwrap().to_string();
    let client = body["clientToken"].as_str().unwrap().to_string();
    assert_eq!(access.len(), 64);
    assert_eq!(client.len(), 64);
    assert_eq!(
        body["selectedProfile"]["id"].as_str(),
        Some(profile_uuid.as_str())
    );
    assert_eq!(body["selectedProfile"]["name"].as_str(), Some("alice"));
    assert!(
        body["user"].is_object(),
        "requestUser=true returns user block"
    );

    // wrong password is rejected with the Mojang error shape (403)
    let (status, body) = post_json(
        &app,
        "/authserver/authenticate",
        json!({ "username": "alice", "password": "wrong" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"].as_str(), Some("ForbiddenOperationException"));
    assert!(body["errorMessage"].is_string());

    // validate → 204
    let (status, _) = post_json(
        &app,
        "/authserver/validate",
        json!({ "accessToken": access, "clientToken": client }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // refresh → new access token, same client token
    let (status, body) = post_json(
        &app,
        "/authserver/refresh",
        json!({ "accessToken": access, "clientToken": client }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "refresh ok: {body}");
    let new_access = body["accessToken"].as_str().unwrap().to_string();
    assert_ne!(new_access, access);
    assert_eq!(body["clientToken"].as_str(), Some(client.as_str()));

    // old access token no longer validates
    let (status, _) = post_json(
        &app,
        "/authserver/validate",
        json!({ "accessToken": access }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // invalidate the new token → 204, then it stops validating
    let (status, _) = post_json(
        &app,
        "/authserver/invalidate",
        json!({ "accessToken": new_access, "clientToken": client }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = post_json(
        &app,
        "/authserver/validate",
        json!({ "accessToken": new_access }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn join_then_has_joined_returns_signed_profile(pool: PgPool) {
    let (uid, profile_uuid) = seed_user(&pool, "bob", "swordfish").await;

    // Give bob a skin so the profile carries a signed textures property.
    sqlx::query(
        "INSERT INTO skins (user_id, profile_uuid, username, file_path, file_url, file_size, variant)
         VALUES ($1, $2, 'bob', 'skins/bob.png', '/textures/skins/bob.png', 1234, 'SLIM')",
    )
    .bind(uid)
    .bind(&profile_uuid)
    .execute(&pool)
    .await
    .unwrap();

    let app = app(pool);

    // authenticate to get an access token
    let (status, body) = post_json(
        &app,
        "/authserver/authenticate",
        json!({ "username": "bob", "password": "swordfish" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let access = body["accessToken"].as_str().unwrap().to_string();

    let server_id = "loontail-test-server-hash";

    // join → 204 (records the in-memory join session)
    let (status, _) = post_json(
        &app,
        "/sessionserver/session/minecraft/join",
        json!({
            "accessToken": access,
            "selectedProfile": profile_uuid,
            "serverId": server_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // hasJoined → signed profile
    let (status, bytes) = get_status_body(
        &app,
        &format!("/sessionserver/session/minecraft/hasJoined?username=bob&serverId={server_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "hasJoined returns a profile");
    let profile: GameProfile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(profile.id, profile_uuid);
    assert_eq!(profile.name, "bob");
    assert_eq!(profile.properties.len(), 1);
    let prop = &profile.properties[0];
    assert_eq!(prop.name, "textures");
    let signature = prop
        .signature
        .as_ref()
        .expect("textures property is signed");

    // The signature verifies against the meta SPKI key with RSA-SHA1.
    let (status, meta_bytes) = get_status_body(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    let meta: Value = serde_json::from_slice(&meta_bytes).unwrap();
    let spki = meta["signaturePublickey"].as_str().unwrap();
    let public = RsaPublicKey::from_public_key_pem(spki).unwrap();
    let sig = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .unwrap();
    let digest = Sha1::digest(prop.value.as_bytes());
    public
        .verify(Pkcs1v15Sign::new::<Sha1>(), &digest, &sig)
        .expect("hasJoined signature verifies against meta SPKI key");

    // The decoded value embeds the absolutized skin URL and the slim model.
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(prop.value.as_bytes())
        .unwrap();
    let value: Value = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(value["profileId"].as_str(), Some(profile_uuid.as_str()));
    assert_eq!(value["profileName"].as_str(), Some("bob"));
    assert_eq!(
        value["textures"]["SKIN"]["url"].as_str(),
        Some("/api/yggdrasil/textures/skins/bob.png")
    );
    assert_eq!(
        value["textures"]["SKIN"]["metadata"]["model"].as_str(),
        Some("slim")
    );

    // The join session is single-use: a second hasJoined returns 204.
    let (status, _) = get_status_body(
        &app,
        &format!("/sessionserver/session/minecraft/hasJoined?username=bob&serverId={server_id}"),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "join session is single-use");
}

#[sqlx::test(migrations = "../../migrations")]
async fn profile_by_uuid_signed_and_unsigned(pool: PgPool) {
    let (uid, profile_uuid) = seed_user(&pool, "carol", "pw").await;
    sqlx::query(
        "INSERT INTO skins (user_id, profile_uuid, username, file_path, file_url, file_size, variant)
         VALUES ($1, $2, 'carol', 'skins/carol.png', '/textures/skins/carol.png', 10, 'CLASSIC')",
    )
    .bind(uid)
    .bind(&profile_uuid)
    .execute(&pool)
    .await
    .unwrap();
    let app = app(pool);

    // signed by default
    let (status, bytes) = get_status_body(
        &app,
        &format!("/sessionserver/session/minecraft/profile/{profile_uuid}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let profile: GameProfile = serde_json::from_slice(&bytes).unwrap();
    assert!(
        profile.properties[0].signature.is_some(),
        "signed by default"
    );

    // ?unsigned=true omits the signature
    let (status, bytes) = get_status_body(
        &app,
        &format!("/sessionserver/session/minecraft/profile/{profile_uuid}?unsigned=true"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let profile: GameProfile = serde_json::from_slice(&bytes).unwrap();
    assert!(
        profile.properties[0].signature.is_none(),
        "unsigned omits the signature"
    );

    // unknown UUID → 204
    let (status, _) = get_status_body(
        &app,
        "/sessionserver/session/minecraft/profile/00000000000000000000000000000000",
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn profiles_by_name_bulk_lookup(pool: PgPool) {
    let (_d, dave_uuid) = seed_user(&pool, "dave", "pw").await;
    seed_user(&pool, "erin", "pw").await;
    let app = app(pool);

    let (status, body) = post_json(
        &app,
        "/api/profiles/minecraft",
        json!(["dave", "nonexistent"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only existing names returned");
    assert_eq!(arr[0]["id"].as_str(), Some(dave_uuid.as_str()));
    assert_eq!(arr[0]["name"].as_str(), Some("dave"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn meta_exposes_spki_public_key(pool: PgPool) {
    let app = app(pool);
    let (status, bytes) = get_status_body(&app, "/").await;
    assert_eq!(status, StatusCode::OK);
    let meta: Value = serde_json::from_slice(&bytes).unwrap();
    let spki = meta["signaturePublickey"].as_str().unwrap();
    assert!(spki.starts_with("-----BEGIN PUBLIC KEY-----"));
    assert!(meta["skinDomains"].is_array());
    assert_eq!(meta["meta"]["serverName"].as_str(), Some("Loontail"));
}
