//! CAT-PERF-1 regression: the catalog list path batches each relation across the
//! whole client set (one `WHERE client_id = ANY($1)` read per relation type) and
//! assembles the DTOs in memory, instead of issuing per-client relation reads in a
//! loop. These tests pin that the batched list output is byte-for-byte the same DTO
//! the per-client `GET /clients/{id}` path returns, and that relations stay correctly
//! attributed per client across a multi-client list (no cross-client leakage).

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
use uuid::Uuid;

fn state(pool: PgPool) -> AppState {
    let config = Config::from_env().expect("config from env");
    AppState::new(pool, config)
}

async fn seed_session_token(pool: &PgPool) -> String {
    let nonce = Uuid::new_v4().simple().to_string();
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

/// Seed a published client with its own slug, en/ru locales, a full media set, one
/// published keyword (en/ru) + one published server, and an owned bundle. Distinct
/// `sort_order` keeps the client list ordering deterministic. Returns the client
/// UUID.
async fn seed_client(pool: &PgPool, slug: &str, sort_order: i32) -> Uuid {
    let client_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_clients \
         (slug, available, minecraft_version, fabric_version, runtime_version, \
          bundle_slug, sort_order, published_at) \
         VALUES ($1, true, '1.21.4', '0.16.0', '21', $2, $3, now()) RETURNING id",
    )
    .bind(slug)
    .bind(format!("{slug}-bundle"))
    .bind(sort_order)
    .fetch_one(pool)
    .await
    .unwrap();

    let bundle_id: Uuid = sqlx::query_scalar(
        "INSERT INTO bundles (slug, name, version, status, files_count) \
         VALUES ($1, $2, '1.0.0', 'ready', $3) RETURNING id",
    )
    .bind(format!("{slug}-bundle"))
    .bind(slug)
    .bind(sort_order + 1)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE catalog_clients SET bundle_id = $1 WHERE id = $2")
        .bind(bundle_id)
        .bind(client_id)
        .execute(pool)
        .await
        .unwrap();

    for (locale, title) in [("en", format!("{slug} EN")), ("ru", format!("{slug} RU"))] {
        sqlx::query(
            "INSERT INTO catalog_client_locales \
             (client_id, locale, title, description, short_description) \
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(client_id)
        .bind(locale)
        .bind(&title)
        .bind(format!("{title} desc"))
        .bind(format!("{title} short"))
        .execute(pool)
        .await
        .unwrap();
    }

    for (role, file) in [
        ("poster", "poster.png"),
        ("background", "bg.png"),
        ("titleImage", "logo.png"),
        ("screenshot", "shot1.png"),
        ("screenshot", "shot2.png"),
    ] {
        sqlx::query(
            "INSERT INTO catalog_media \
             (client_id, role, url, ext, mime, width, height, size) \
             VALUES ($1,$2,$3,'png','image/png',1920,1080,5000)",
        )
        .bind(client_id)
        .bind(role)
        .bind(format!("/catalog-media/{slug}/{file}"))
        .execute(pool)
        .await
        .unwrap();
    }

    let keyword_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_keywords (slug, published_at) VALUES ($1, now()) RETURNING id",
    )
    .bind(format!("{slug}-kw"))
    .fetch_one(pool)
    .await
    .unwrap();
    for (locale, title) in [
        ("en", format!("{slug}-kw EN")),
        ("ru", format!("{slug}-kw RU")),
    ] {
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

    let server_id: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog_servers (slug, name, address, published_at) \
         VALUES ($1, $2, $3, now()) RETURNING id",
    )
    .bind(format!("{slug}-srv"))
    .bind(format!("{slug} server"))
    .bind(format!("play.{slug}.example"))
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

/// The batched list DTO for each client must be byte-identical to the per-client
/// `GET /clients/{id}` DTO. This is the contract proof: the N+1 → batch refactor
/// changed only how relations are fetched, never the serialized shape/values.
#[sqlx::test(migrations = "../../migrations")]
async fn list_dtos_match_per_client_get(pool: PgPool) {
    let mut ids = Vec::new();
    for (i, slug) in ["alpha", "bravo", "charlie"].into_iter().enumerate() {
        ids.push(seed_client(&pool, slug, i as i32).await);
    }

    let (status, body) = get_json(pool.clone(), "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    let clients = body
        .get("clients")
        .and_then(Value::as_array)
        .expect("clients array");
    assert_eq!(clients.len(), 3, "all three published clients listed");

    // Each client in the list must equal the single-fetch DTO for the same id.
    for id in &ids {
        let doc = id.simple().to_string();
        let (gstatus, gbody) = get_json(pool.clone(), &format!("/clients/{doc}?locale=en")).await;
        assert_eq!(gstatus, StatusCode::OK);

        let from_list = clients
            .iter()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(doc.as_str()))
            .unwrap_or_else(|| panic!("client {doc} present in list"));
        assert_eq!(
            from_list, &gbody,
            "list DTO must equal the per-client GET DTO for {doc}"
        );
    }
}

/// Relations stay correctly attributed per client across the batched list: each
/// client carries exactly its own keyword/server/media/bundle, with zero leakage
/// from the other clients in the same `ANY($1)` read.
#[sqlx::test(migrations = "../../migrations")]
async fn list_relations_are_partitioned_per_client(pool: PgPool) {
    for (i, slug) in ["alpha", "bravo", "charlie"].into_iter().enumerate() {
        seed_client(&pool, slug, i as i32).await;
    }

    let (status, body) = get_json(pool, "/clients?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    let clients = body.get("clients").and_then(Value::as_array).unwrap();
    assert_eq!(clients.len(), 3);

    // List ordering follows sort_order: alpha, bravo, charlie.
    let slugs: Vec<&str> = clients
        .iter()
        .map(|c| c.get("slug").and_then(Value::as_str).unwrap())
        .collect();
    assert_eq!(slugs, ["alpha", "bravo", "charlie"]);

    for slug in ["alpha", "bravo", "charlie"] {
        let client = clients
            .iter()
            .find(|c| c.get("slug").and_then(Value::as_str) == Some(slug))
            .unwrap();

        // Exactly one keyword/server, both this client's own (en title resolved).
        let keywords = client.get("keywords").and_then(Value::as_array).unwrap();
        assert_eq!(keywords.len(), 1, "{slug}: exactly its own keyword");
        assert_eq!(
            keywords[0].get("title").and_then(Value::as_str),
            Some(format!("{slug}-kw EN").as_str())
        );

        let servers = client.get("servers").and_then(Value::as_array).unwrap();
        assert_eq!(servers.len(), 1, "{slug}: exactly its own server");
        assert_eq!(
            servers[0].get("address").and_then(Value::as_str),
            Some(format!("play.{slug}.example").as_str())
        );

        // Media role slots + screenshots belong to this client only.
        assert_eq!(
            client.pointer("/poster/url").and_then(Value::as_str),
            Some(format!("/catalog-media/{slug}/poster.png").as_str())
        );
        assert_eq!(
            client.pointer("/background/url").and_then(Value::as_str),
            Some(format!("/catalog-media/{slug}/bg.png").as_str())
        );
        assert_eq!(
            client.pointer("/titleImage/url").and_then(Value::as_str),
            Some(format!("/catalog-media/{slug}/logo.png").as_str())
        );
        let shots = client.get("screenshots").and_then(Value::as_array).unwrap();
        assert_eq!(shots.len(), 2, "{slug}: its own two screenshots");
        assert_eq!(
            shots[0].get("url").and_then(Value::as_str),
            Some(format!("/catalog-media/{slug}/shot1.png").as_str())
        );

        // Localized text + owned bundle are this client's.
        assert_eq!(
            client.get("title").and_then(Value::as_str),
            Some(format!("{slug} EN").as_str())
        );
        assert_eq!(
            client.pointer("/bundle/slug").and_then(Value::as_str),
            Some(format!("{slug}-bundle").as_str())
        );
        assert_eq!(
            client.get("bundleSlug").and_then(Value::as_str),
            Some(format!("{slug}-bundle").as_str())
        );
    }
}
