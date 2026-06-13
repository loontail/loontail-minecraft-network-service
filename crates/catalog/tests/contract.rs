//! Contract tests for the launcher catalog. Each test seeds a published client
//! with locales, media, keywords, and servers, then drives the public
//! `routes()` via `tower::ServiceExt::oneshot` against a per-test database
//! (`#[sqlx::test]`). Assertions pin the exact JSON field names + envelope shape
//! the launcher's `normalizeClient`/Strapi schemas consume.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use loontail_core::{AppState, Config};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const POPULATE: &str = "populate[screenshots]=true&populate[background]=true&\
populate[poster]=true&populate[titleImage]=true&populate[keywords]=true&populate[servers]=true";

fn state(pool: PgPool) -> AppState {
    // Config::from_env reads DATABASE_URL (set for the test) + defaults for the rest.
    let config = Config::from_env().expect("config from env");
    AppState::new(pool, config)
}

async fn get_json(pool: PgPool, uri: &str) -> (StatusCode, Value) {
    let app = loontail_catalog::routes().with_state(state(pool));
    let res = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
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
        ("poster", "/uploads/poster.png"),
        ("background", "/uploads/bg.png"),
        ("titleImage", "/uploads/logo.png"),
        ("screenshot", "/uploads/shot1.png"),
    ] {
        sqlx::query(
            "INSERT INTO catalog_media \
             (client_id, role, url, ext, name, hash, mime, width, height, size, formats) \
             VALUES ($1,$2,$3,'.png','img','abc','image/png',1920,1080,5000, \
             '{\"thumbnail\":{\"url\":\"/uploads/thumb.png\",\"ext\":\".png\",\"width\":16,\"height\":9,\"size\":10,\"name\":\"t\",\"hash\":\"h\"}}'::jsonb)",
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
async fn clients_envelope_and_populated_relations(pool: PgPool) {
    seed_published_client(&pool).await;
    let uri = format!("/clients?{POPULATE}&locale=en");
    let (status, body) = get_json(pool, &uri).await;
    assert_eq!(status, StatusCode::OK);

    // Envelope shape.
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .expect("data array");
    assert_eq!(data.len(), 1);
    let pagination = body
        .pointer("/meta/pagination")
        .expect("meta.pagination present");
    assert_eq!(pagination.get("page").and_then(Value::as_i64), Some(1));
    assert_eq!(pagination.get("pageCount").and_then(Value::as_i64), Some(1));
    assert_eq!(pagination.get("total").and_then(Value::as_i64), Some(1));
    assert!(pagination.get("pageSize").and_then(Value::as_i64).is_some());

    let client = &data[0];
    // Exact field names the launcher's ClientResponseSchema/normalizeClient expect.
    assert!(client.get("id").and_then(Value::as_i64).is_some());
    assert!(client.get("documentId").and_then(Value::as_str).is_some());
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
    // Nullable version field stays null (forgeVersion was NULL).
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
    assert!(client.get("createdAt").and_then(Value::as_str).is_some());
    assert!(client.get("updatedAt").and_then(Value::as_str).is_some());
    assert!(client.get("publishedAt").and_then(Value::as_str).is_some());

    // Populated media: server-relative url (launcher absolutizes).
    let poster = client.get("poster").expect("poster present");
    assert_eq!(
        poster.get("url").and_then(Value::as_str),
        Some("/uploads/poster.png")
    );
    assert!(poster.get("id").and_then(Value::as_i64).is_some());
    assert!(poster.get("formats").is_some());
    let background = client.get("background").expect("background present");
    assert_eq!(
        background.get("url").and_then(Value::as_str),
        Some("/uploads/bg.png")
    );
    let title_image = client.get("titleImage").expect("titleImage present");
    assert_eq!(
        title_image.get("url").and_then(Value::as_str),
        Some("/uploads/logo.png")
    );
    let screenshots = client
        .get("screenshots")
        .and_then(Value::as_array)
        .expect("screenshots array");
    assert_eq!(screenshots.len(), 1);
    assert_eq!(
        screenshots[0].get("url").and_then(Value::as_str),
        Some("/uploads/shot1.png")
    );

    // Populated keywords with localized title.
    let keywords = client
        .get("keywords")
        .and_then(Value::as_array)
        .expect("keywords array");
    assert_eq!(keywords.len(), 1);
    assert_eq!(
        keywords[0].get("title").and_then(Value::as_str),
        Some("Survival")
    );

    // Populated servers.
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
    let (_, body) = get_json(pool.clone(), &format!("/clients?{POPULATE}&locale=ru")).await;
    let client = &body.pointer("/data/0").unwrap();
    assert_eq!(client.get("title").and_then(Value::as_str), Some("Аврора"));
    assert_eq!(
        client.pointer("/keywords/0/title").and_then(Value::as_str),
        Some("Выживание")
    );

    // Requested an unknown locale → falls back to default ("en").
    let (_, body) = get_json(pool, &format!("/clients?{POPULATE}&locale=fr")).await;
    let client = &body.pointer("/data/0").unwrap();
    assert_eq!(client.get("title").and_then(Value::as_str), Some("Aurora"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn draft_client_hidden_from_public(pool: PgPool) {
    // A draft client (published_at NULL) must not appear in public reads.
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

    let (status, body) = get_json(pool, &format!("/clients?{POPULATE}&locale=en")).await;
    assert_eq!(status, StatusCode::OK);
    let data = body.get("data").and_then(Value::as_array).unwrap();
    assert!(data.is_empty(), "draft client must be hidden");
    assert_eq!(
        body.pointer("/meta/pagination/total")
            .and_then(Value::as_i64),
        Some(0)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn unpopulated_relations_are_empty(pool: PgPool) {
    seed_published_client(&pool).await;
    // No populate params → relations default empty/null (Strapi behavior).
    let (status, body) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    let client = body.pointer("/data/0").unwrap();
    assert!(client.get("poster").unwrap().is_null());
    assert!(client.get("background").unwrap().is_null());
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
    // titleImage is optional ⇒ absent entirely when not populated.
    assert!(client.get("titleImage").is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_client_by_id_document_id_and_slug(pool: PgPool) {
    let uuid = seed_published_client(&pool).await;

    // By slug.
    let (status, body) = get_json(pool.clone(), &format!("/clients/aurora?{POPULATE}")).await;
    assert_eq!(status, StatusCode::OK);
    let seq = body.pointer("/data/id").and_then(Value::as_i64).unwrap();
    assert_eq!(
        body.pointer("/data/slug").and_then(Value::as_str),
        Some("aurora")
    );

    // By documentId (undashed UUID).
    let doc = uuid.simple().to_string();
    let (status, body) = get_json(pool.clone(), &format!("/clients/{doc}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.pointer("/data/documentId").and_then(Value::as_str),
        Some(doc.as_str())
    );

    // By numeric id.
    let (status, body) = get_json(pool, &format!("/clients/{seq}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.pointer("/data/id").and_then(Value::as_i64), Some(seq));
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
    let data = body.get("data").and_then(Value::as_array).unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0].get("title").and_then(Value::as_str),
        Some("Выживание")
    );
    assert_eq!(
        body.pointer("/meta/pagination/total")
            .and_then(Value::as_i64),
        Some(1)
    );

    let (status, body) = get_json(pool, "/servers").await;
    assert_eq!(status, StatusCode::OK);
    let data = body.get("data").and_then(Value::as_array).unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(
        data[0].get("address").and_then(Value::as_str),
        Some("play.loontail.com")
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn empty_bundle_slug_collapses_to_null(pool: PgPool) {
    sqlx::query(
        "INSERT INTO catalog_clients (slug, available, bundle_slug, published_at) \
         VALUES ('nobundle', true, '   ', now())",
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

    let (_, body) = get_json(pool, "/clients?locale=en").await;
    let client = body.pointer("/data/0").unwrap();
    assert!(
        client.get("bundleSlug").unwrap().is_null(),
        "whitespace-only bundleSlug must collapse to null"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn invalid_api_token_is_rejected_valid_or_absent_allowed(pool: PgPool) {
    seed_published_client(&pool).await;

    // A valid API token (sha-256 of "secret-token") is accepted.
    let hash = loontail_catalog::hash_api_token("secret-token");
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('launcher', $1)")
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();

    let app = loontail_catalog::routes().with_state(state(pool.clone()));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients?locale=en")
                .header("authorization", "Bearer secret-token")
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

    // No token ⇒ public read still allowed.
    let (status, _) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
}
