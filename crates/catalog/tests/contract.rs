//! Contract tests for the launcher catalog. Each test seeds a published client
//! with locales, media, keywords, and servers, then drives the public
//! `routes()` via `tower::ServiceExt::oneshot` against a per-test database
//! (`#[sqlx::test]`). Assertions pin the exact native JSON field names + flat
//! shape (no envelopes, relations always inlined) the launcher consumes.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use loontail_core::auth::issue_session;
use loontail_core::identity::register_user;
use loontail_core::{AppState, Config};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

fn state(pool: PgPool) -> AppState {
    // Config::from_env reads DATABASE_URL (set for the test) + defaults for the rest.
    let config = Config::from_env().expect("config from env");
    AppState::new(pool, config)
}

/// Create a fresh non-admin account and mint a session, returning its raw token.
/// Every public catalog read requires a valid `AuthUser`, so the launcher attaches
/// this as a Bearer token; the user is unrelated to the seeded catalog rows, so it
/// does not perturb any assertion.
async fn seed_session_token(pool: &PgPool) -> String {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let user = register_user(
        pool,
        &format!("reader-{nonce}"),
        &format!("reader-{nonce}@example.com"),
        "pw",
    )
    .await
    .expect("register catalog reader");
    issue_session(pool, user.id, Duration::from_secs(900))
        .await
        .expect("issue session")
        .token
}

async fn get_json(pool: PgPool, uri: &str) -> (StatusCode, Value) {
    let token = seed_session_token(&pool).await;
    let app = loontail_catalog::routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, json)
}

/// Seed one published client (slug `aurora`) with: en+ru locales, a poster,
/// background, titleImage, one screenshot, a published keyword (en+ru), and a
/// published server. Returns the client UUID.
async fn seed_published_client(pool: &PgPool) -> uuid::Uuid {
    let client_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients \
         (slug, available, minecraft_version, forge_version, fabric_version, \
          runtime_version, bundle_slug, published_at) \
         VALUES ('aurora', true, '1.21.4', NULL, '0.16.0', '21', 'aurora-bundle', now()) \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    // Owned bundle for the build, linked via the real FK. The client's text
    // `bundle_slug` stays for the frozen `bundleSlug` contract; `bundle_id` drives
    // the additive inlined `bundle` summary.
    let bundle_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO bundles (slug, name, version, status, files_count) \
         VALUES ('aurora-bundle', 'Aurora', '1.0.0', 'ready', 3) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE catalog_clients SET bundle_id = $1 WHERE id = $2")
        .bind(bundle_id)
        .bind(client_id)
        .execute(pool)
        .await
        .unwrap();

    for (locale, title, desc, short) in [
        ("en", "Aurora", "The Aurora build", "Short EN"),
        ("ru", "Аврора", "Сборка Аврора", "Кратко RU"),
    ] {
        sqlx::query(
            "INSERT INTO catalog_client_locales \
             (client_id, locale, title, description, short_description) \
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(client_id)
        .bind(locale)
        .bind(title)
        .bind(desc)
        .bind(short)
        .execute(pool)
        .await
        .unwrap();
    }

    for (role, url) in [
        ("poster", "/catalog-media/aurora/poster.png"),
        ("background", "/catalog-media/aurora/bg.png"),
        ("titleImage", "/catalog-media/aurora/logo.png"),
        ("screenshot", "/catalog-media/aurora/shot1.png"),
    ] {
        sqlx::query(
            "INSERT INTO catalog_media \
             (client_id, role, url, ext, mime, width, height, size) \
             VALUES ($1,$2,$3,'png','image/png',1920,1080,5000)",
        )
        .bind(client_id)
        .bind(role)
        .bind(url)
        .execute(pool)
        .await
        .unwrap();
    }

    let keyword_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_keywords (slug, published_at) VALUES ('survival', now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    for (locale, title) in [("en", "Survival"), ("ru", "Выживание")] {
        sqlx::query(
            "INSERT INTO catalog_keyword_locales (keyword_id, locale, title) VALUES ($1,$2,$3)",
        )
        .bind(keyword_id)
        .bind(locale)
        .bind(title)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO catalog_client_keywords (client_id, keyword_id) VALUES ($1,$2)")
        .bind(client_id)
        .bind(keyword_id)
        .execute(pool)
        .await
        .unwrap();

    let server_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_servers (slug, name, address, published_at) \
         VALUES ('main', 'Main', 'play.loontail.com', now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO catalog_client_servers (client_id, server_id) VALUES ($1,$2)")
        .bind(client_id)
        .bind(server_id)
        .execute(pool)
        .await
        .unwrap();

    client_id
}

#[sqlx::test(migrations = "../../migrations")]
async fn clients_native_list_and_inlined_relations(pool: PgPool) {
    let client_id = seed_published_client(&pool).await;
    let (status, body) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);

    // Native list wrapper: { clients: [...] } — no `data`/`meta` envelope.
    assert!(
        body.get("data").is_none(),
        "native shape must not carry `data`"
    );
    assert!(
        body.get("meta").is_none(),
        "native shape must not carry `meta`"
    );
    let clients = body
        .get("clients")
        .and_then(Value::as_array)
        .expect("clients array");
    assert_eq!(clients.len(), 1);

    let client = &clients[0];
    // `id` is the undashed UUID; no documentId/seq/createdAt/updatedAt/publishedAt.
    assert_eq!(
        client.get("id").and_then(Value::as_str),
        Some(client_id.simple().to_string().as_str())
    );
    assert!(client.get("documentId").is_none());
    assert!(client.get("createdAt").is_none());
    assert!(client.get("updatedAt").is_none());
    assert!(client.get("publishedAt").is_none());

    assert_eq!(client.get("slug").and_then(Value::as_str), Some("aurora"));
    assert_eq!(client.get("title").and_then(Value::as_str), Some("Aurora"));
    assert_eq!(
        client.get("description").and_then(Value::as_str),
        Some("The Aurora build")
    );
    assert_eq!(
        client.get("shortDescription").and_then(Value::as_str),
        Some("Short EN")
    );
    assert_eq!(client.get("available").and_then(Value::as_bool), Some(true));
    assert_eq!(
        client.get("minecraftVersion").and_then(Value::as_str),
        Some("1.21.4")
    );
    assert!(client.get("forgeVersion").unwrap().is_null());
    assert_eq!(
        client.get("fabricVersion").and_then(Value::as_str),
        Some("0.16.0")
    );
    assert_eq!(
        client.get("runtimeVersion").and_then(Value::as_str),
        Some("21")
    );
    assert_eq!(
        client.get("bundleSlug").and_then(Value::as_str),
        Some("aurora-bundle")
    );

    // Additive inlined bundle summary from the linked `bundle_id`. `bundleSlug`
    // (asserted above) MUST still be present alongside it.
    let bundle = client.get("bundle").expect("bundle present");
    assert_eq!(
        bundle.get("slug").and_then(Value::as_str),
        Some("aurora-bundle")
    );
    assert_eq!(bundle.get("status").and_then(Value::as_str), Some("ready"));
    assert_eq!(bundle.get("version").and_then(Value::as_str), Some("1.0.0"));
    assert_eq!(bundle.get("filesCount").and_then(Value::as_i64), Some(3));
    assert_eq!(
        bundle.get("manifestUrl").and_then(Value::as_str),
        Some("/api/bundle-registry/builds/aurora-bundle/manifest")
    );

    // Media: { url, width, height } only — no id/ext/name/hash/size/formats.
    let poster = client.get("poster").expect("poster present");
    assert_eq!(
        poster.get("url").and_then(Value::as_str),
        Some("/catalog-media/aurora/poster.png")
    );
    assert_eq!(poster.get("width").and_then(Value::as_i64), Some(1920));
    assert_eq!(poster.get("height").and_then(Value::as_i64), Some(1080));
    assert!(poster.get("id").is_none());
    assert!(poster.get("ext").is_none());
    assert!(poster.get("formats").is_none());

    let background = client.get("background").expect("background present");
    assert_eq!(
        background.get("url").and_then(Value::as_str),
        Some("/catalog-media/aurora/bg.png")
    );
    let title_image = client.get("titleImage").expect("titleImage present");
    assert_eq!(
        title_image.get("url").and_then(Value::as_str),
        Some("/catalog-media/aurora/logo.png")
    );
    let screenshots = client
        .get("screenshots")
        .and_then(Value::as_array)
        .expect("screenshots array");
    assert_eq!(screenshots.len(), 1);
    assert_eq!(
        screenshots[0].get("url").and_then(Value::as_str),
        Some("/catalog-media/aurora/shot1.png")
    );

    // Keywords are always inlined: { id (undashed uuid), title }.
    let keywords = client
        .get("keywords")
        .and_then(Value::as_array)
        .expect("keywords array");
    assert_eq!(keywords.len(), 1);
    assert_eq!(
        keywords[0].get("title").and_then(Value::as_str),
        Some("Survival")
    );
    let kw_id = keywords[0].get("id").and_then(Value::as_str).unwrap();
    assert_eq!(kw_id.len(), 32, "keyword id is undashed 32-char hex");

    // Servers are always inlined: { id, name, address }.
    let servers = client
        .get("servers")
        .and_then(Value::as_array)
        .expect("servers array");
    assert_eq!(servers.len(), 1);
    assert_eq!(
        servers[0].get("address").and_then(Value::as_str),
        Some("play.loontail.com")
    );
    assert_eq!(servers[0].get("name").and_then(Value::as_str), Some("Main"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn locale_fallback_to_default_then_any(pool: PgPool) {
    seed_published_client(&pool).await;

    // Requested ru → ru title.
    let (_, body) = get_json(pool.clone(), "/clients?locale=ru").await;
    let client = &body.pointer("/clients/0").unwrap();
    assert_eq!(client.get("title").and_then(Value::as_str), Some("Аврора"));
    assert_eq!(
        client.pointer("/keywords/0/title").and_then(Value::as_str),
        Some("Выживание")
    );

    // Requested an unknown locale → falls back to default ("en").
    let (_, body) = get_json(pool, "/clients?locale=fr").await;
    let client = &body.pointer("/clients/0").unwrap();
    assert_eq!(client.get("title").and_then(Value::as_str), Some("Aurora"));
}

/// A repeated `?locale=` must be LAST-WINS, not a hard error. axum's `Query`
/// (serde_urlencoded) reports `duplicate field` and answers with a PLAIN-TEXT 400 that
/// bypasses the `{error:{code,message}}` envelope, so any URL builder or intermediary
/// that appends the param twice would break every catalog read for that client.
#[sqlx::test(migrations = "../../migrations")]
async fn repeated_locale_takes_the_last_value(pool: PgPool) {
    seed_published_client(&pool).await;
    let token = seed_session_token(&pool).await;
    let app = loontail_catalog::routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients?locale=en&locale=ru")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "a duplicated query key must not 400: {}",
        String::from_utf8_lossy(&bytes)
    );
    let body: Value = serde_json::from_slice(&bytes).expect("JSON body");
    assert_eq!(
        body.pointer("/clients/0/title").and_then(Value::as_str),
        Some("Аврора"),
        "the last locale wins"
    );
}

/// GET through a router that nests the catalog under `/api`, exactly as the server does.
async fn nested_get_json(pool: PgPool, uri: &str) -> Value {
    let token = seed_session_token(&pool).await;
    let app = axum::Router::new()
        .nest("/api", loontail_catalog::routes())
        .with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "{uri}");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// BEQ-18(c): `locale` comes from a `CatalogQuery` extractor reading `parts.uri`, not
/// from `OriginalUri`. The live router serves these routes NESTED under `/api`, and
/// nesting rewrites the request path — so pin that the query string still reaches the
/// extractor there, or every launcher request would silently fall back to English.
#[sqlx::test(migrations = "../../migrations")]
async fn locale_survives_the_api_nesting(pool: PgPool) {
    seed_published_client(&pool).await;

    let body = nested_get_json(pool.clone(), "/api/clients?locale=ru").await;
    assert_eq!(
        body.pointer("/clients/0/title").and_then(Value::as_str),
        Some("Аврора"),
        "the nested route still sees ?locale=ru"
    );
    assert_eq!(
        body.pointer("/clients/0/keywords/0/title")
            .and_then(Value::as_str),
        Some("Выживание"),
        "and the batched keyword titles honour it"
    );

    let body = nested_get_json(pool.clone(), "/api/keywords?locale=ru").await;
    assert_eq!(
        body.pointer("/keywords/0/title").and_then(Value::as_str),
        Some("Выживание"),
        "the batched list_keywords path honours it too"
    );

    let body = nested_get_json(pool, "/api/clients").await;
    assert_eq!(
        body.pointer("/clients/0/title").and_then(Value::as_str),
        Some("Aurora"),
        "absent locale falls back to the default"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn draft_client_hidden_from_public(pool: PgPool) {
    sqlx::query(
        "INSERT INTO catalog_clients (slug, available, published_at) VALUES ('hidden', true, NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) \
         SELECT id, 'en', 'Hidden' FROM catalog_clients WHERE slug = 'hidden'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    let clients = body.get("clients").and_then(Value::as_array).unwrap();
    assert!(clients.is_empty(), "draft client must be hidden");
}

#[sqlx::test(migrations = "../../migrations")]
async fn relations_default_empty_when_absent(pool: PgPool) {
    // A client with no media/keywords/servers gets null slots + empty arrays.
    sqlx::query(
        "INSERT INTO catalog_clients (slug, available, published_at) VALUES ('bare', true, now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) \
         SELECT id, 'en', 'Bare' FROM catalog_clients WHERE slug = 'bare'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    let client = body.pointer("/clients/0").unwrap();
    assert!(client.get("poster").unwrap().is_null());
    assert!(client.get("background").unwrap().is_null());
    assert!(client.get("titleImage").unwrap().is_null());
    assert!(client
        .get("screenshots")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
    assert!(client
        .get("keywords")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
    assert!(client
        .get("servers")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
    // A client with no linked bundle inlines `bundle: null`.
    assert!(
        client.get("bundle").unwrap().is_null(),
        "client without a bundle link must have null bundle"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_client_by_id_and_slug(pool: PgPool) {
    let uuid = seed_published_client(&pool).await;
    let doc = uuid.simple().to_string();

    // By slug — returns the bare client object (no envelope).
    let (status, body) = get_json(pool.clone(), "/clients/aurora").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("data").is_none());
    assert_eq!(body.get("slug").and_then(Value::as_str), Some("aurora"));
    assert_eq!(body.get("id").and_then(Value::as_str), Some(doc.as_str()));

    // By undashed UUID id.
    let (status, body) = get_json(pool, &format!("/clients/{doc}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("id").and_then(Value::as_str), Some(doc.as_str()));
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_missing_client_is_404(pool: PgPool) {
    let (status, _) = get_json(pool, "/clients/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../migrations")]
async fn keywords_and_servers_lists(pool: PgPool) {
    seed_published_client(&pool).await;

    let (status, body) = get_json(pool.clone(), "/keywords?locale=ru").await;
    assert_eq!(status, StatusCode::OK);
    let keywords = body.get("keywords").and_then(Value::as_array).unwrap();
    assert_eq!(keywords.len(), 1);
    assert_eq!(
        keywords[0].get("title").and_then(Value::as_str),
        Some("Выживание")
    );

    let (status, body) = get_json(pool, "/servers").await;
    assert_eq!(status, StatusCode::OK);
    let servers = body.get("servers").and_then(Value::as_array).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(
        servers[0].get("address").and_then(Value::as_str),
        Some("play.loontail.com")
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn dangling_bundle_slug_reads_null(pool: PgPool) {
    // A published client carries a non-empty `bundle_slug` column but `bundle_id`
    // is NULL (dangling/never-linked: the named bundle row does not exist). The wire
    // `bundleSlug` is sourced from the VERIFIED bundle, so it reads null and the
    // inlined `bundle` is null — no consumer chases a slug with no bundle behind it.
    sqlx::query(
        "INSERT INTO catalog_clients (slug, available, bundle_slug, published_at) \
         VALUES ('nobundle', true, 'ghost-bundle', now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) \
         SELECT id, 'en', 'No Bundle' FROM catalog_clients WHERE slug = 'nobundle'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    let client = body.pointer("/clients/0").unwrap();
    assert!(
        client.get("bundleSlug").unwrap().is_null(),
        "dangling bundleSlug (no matching bundle row) must read null"
    );
    assert!(
        client.get("bundle").unwrap().is_null(),
        "dangling bundle link must inline bundle: null"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_required_valid_accepted_invalid_and_absent_rejected(pool: PgPool) {
    seed_published_client(&pool).await;

    // A valid session Bearer token is accepted.
    let token = seed_session_token(&pool).await;
    let app = loontail_catalog::routes().with_state(state(pool.clone()));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients?locale=en")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // An unknown token is rejected.
    let app = loontail_catalog::routes().with_state(state(pool.clone()));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients?locale=en")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // No token ⇒ rejected (there is no anonymous catalog read).
    let app = loontail_catalog::routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients?locale=en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
