//! Admin catalog list contract: `GET /clients` on `admin_routes()` returns ALL
//! clients including drafts, each with its real `published` flag — the surface
//! the admin SPA needs to manage freshly created (unpublished) builds. The public
//! list (`contract.rs`) keeps the draft filter, so unpublished builds stay hidden
//! from the launcher.

use sqlx::AssertSqlSafe;
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

fn state(pool: PgPool) -> AppState {
    let config = Config::from_env().expect("config from env");
    AppState::new(pool, config)
}

/// Register a user, promote it to `is_admin`, and mint a session Bearer token.
async fn seed_admin_token(pool: &PgPool) -> String {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
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

async fn admin_get(pool: PgPool, uri: &str, token: &str) -> (StatusCode, Value) {
    let app = loontail_catalog::admin_routes().with_state(state(pool));
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

async fn insert_client(pool: &PgPool, slug: &str, published: bool) {
    let published_at = if published { "now()" } else { "NULL" };
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO catalog_clients (slug, available, published_at) \
         VALUES ($1, true, {published_at})"
    )))
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) \
         SELECT id, 'en', $2 FROM catalog_clients WHERE slug = $1",
    )
    .bind(slug)
    .bind(slug)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_list_includes_drafts_with_published_flag(pool: PgPool) {
    insert_client(&pool, "live-build", true).await;
    insert_client(&pool, "draft-build", false).await;
    let token = seed_admin_token(&pool).await;

    let (status, body) = admin_get(pool, "/clients?locale=en", &token).await;
    assert_eq!(status, StatusCode::OK);

    let clients = body
        .get("clients")
        .and_then(Value::as_array)
        .expect("clients array");
    assert_eq!(clients.len(), 2, "admin list must include the draft");

    let by_slug = |slug: &str| -> Value {
        clients
            .iter()
            .find(|c| c.get("slug").and_then(Value::as_str) == Some(slug))
            .unwrap_or_else(|| panic!("{slug} present"))
            .clone()
    };
    let live = by_slug("live-build");
    let draft = by_slug("draft-build");
    assert_eq!(live.get("published").and_then(Value::as_bool), Some(true));
    assert_eq!(draft.get("published").and_then(Value::as_bool), Some(false));

    // Still the native flat shape: no envelope, undashed id, inlined relations.
    assert!(body.get("data").is_none());
    assert_eq!(
        live.get("id").and_then(Value::as_str).map(str::len),
        Some(32)
    );
    assert!(live
        .get("screenshots")
        .and_then(Value::as_array)
        .unwrap()
        .is_empty());
}

async fn admin_patch(pool: PgPool, id: uuid::Uuid, token: &str, body: &Value) -> StatusCode {
    let app = loontail_catalog::admin_routes().with_state(state(pool));
    app.oneshot(
        Request::builder()
            .method("PATCH")
            .uri(format!("/clients/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

async fn client_id_by_slug(pool: &PgPool, slug: &str) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM catalog_clients WHERE slug = $1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// BEQ-E1: `sortOrder` is absent from every save the admin SPA sends, and the read DTO
/// does not expose it — so an update that omits it must leave the stored order alone.
/// It used to bind a serde default of 0, silently reordering the launcher's client list.
#[sqlx::test(migrations = "../../migrations")]
async fn update_without_sort_order_preserves_it(pool: PgPool) {
    insert_client(&pool, "ordered-build", true).await;
    let id = client_id_by_slug(&pool, "ordered-build").await;
    sqlx::query("UPDATE catalog_clients SET sort_order = 5 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let token = seed_admin_token(&pool).await;

    let body = json!({
        "slug": "ordered-build",
        "available": true,
        "locales": [{ "locale": "en", "title": "Ordered" }],
    });
    assert_eq!(
        admin_patch(pool.clone(), id, &token, &body).await,
        StatusCode::OK
    );

    let sort_order: i32 =
        sqlx::query_scalar("SELECT sort_order FROM catalog_clients WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        sort_order, 5,
        "an omitted sortOrder must not reset the column"
    );

    // An explicit value still writes through.
    let body = json!({
        "slug": "ordered-build",
        "available": true,
        "sortOrder": 9,
        "locales": [{ "locale": "en", "title": "Ordered" }],
    });
    assert_eq!(
        admin_patch(pool.clone(), id, &token, &body).await,
        StatusCode::OK
    );
    let sort_order: i32 =
        sqlx::query_scalar("SELECT sort_order FROM catalog_clients WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sort_order, 9);
}

/// BEQ-18(a): the admin SPA always submits exactly one `en` locale, and update used to
/// DELETE every locale row before rewriting the payload — silently destroying any
/// ru/de/... translation on every save.
#[sqlx::test(migrations = "../../migrations")]
async fn update_with_one_locale_keeps_the_others(pool: PgPool) {
    insert_client(&pool, "multi-locale", true).await;
    let id = client_id_by_slug(&pool, "multi-locale").await;
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) VALUES ($1, 'ru', 'Русский')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let token = seed_admin_token(&pool).await;

    let body = json!({
        "slug": "multi-locale",
        "available": true,
        "locales": [{ "locale": "en", "title": "English updated" }],
    });
    assert_eq!(
        admin_patch(pool.clone(), id, &token, &body).await,
        StatusCode::OK
    );

    let titles: Vec<(String, String)> = sqlx::query_as(
        "SELECT locale, title FROM catalog_client_locales WHERE client_id = $1 ORDER BY locale",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        titles,
        vec![
            ("en".to_string(), "English updated".to_string()),
            ("ru".to_string(), "Русский".to_string()),
        ],
        "en is upserted, ru survives"
    );
}

/// `replaceLocales: true` is the explicit opt-in that makes the payload authoritative.
#[sqlx::test(migrations = "../../migrations")]
async fn replace_locales_opt_in_drops_unlisted_locales(pool: PgPool) {
    insert_client(&pool, "replace-locales", true).await;
    let id = client_id_by_slug(&pool, "replace-locales").await;
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) VALUES ($1, 'ru', 'Русский')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let token = seed_admin_token(&pool).await;

    let body = json!({
        "slug": "replace-locales",
        "available": true,
        "replaceLocales": true,
        "locales": [{ "locale": "en", "title": "Only English" }],
    });
    assert_eq!(
        admin_patch(pool.clone(), id, &token, &body).await,
        StatusCode::OK
    );

    let locales: Vec<String> =
        sqlx::query_scalar("SELECT locale FROM catalog_client_locales WHERE client_id = $1")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(locales, vec!["en".to_string()], "ru dropped on opt-in");
}

/// BEQ-18(c): the hand-rolled percent-decoder was replaced by axum's `Query` extractor.
/// These are the decoder's own unit tests, re-pointed at the real route: a full `%XX`
/// escape at the end of the value decodes, and `?locale=` reads as absent (falling back
/// to the default locale rather than matching a locale literally named `""`).
#[sqlx::test(migrations = "../../migrations")]
async fn locale_query_is_percent_decoded_and_empty_is_absent(pool: PgPool) {
    insert_client(&pool, "loc-build", true).await;
    let id = client_id_by_slug(&pool, "loc-build").await;
    sqlx::query(
        "INSERT INTO catalog_client_locales (client_id, locale, title) \
         VALUES ($1, 'en-us', 'American')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let token = seed_admin_token(&pool).await;

    let title_for = |uri: &'static str, pool: PgPool, token: String| async move {
        let (status, body) = admin_get(pool, uri, &token).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        body["clients"][0]["title"].as_str().unwrap().to_string()
    };

    assert_eq!(
        title_for("/clients?locale=en%2Dus", pool.clone(), token.clone()).await,
        "American",
        "%2D decodes to '-' so the en-us row matches"
    );
    // `insert_client` seeded the `en` row titled after the slug.
    assert_eq!(
        title_for("/clients?locale=", pool.clone(), token.clone()).await,
        "loc-build",
        "an empty locale value falls back to the default locale"
    );
    assert_eq!(
        title_for("/clients", pool.clone(), token.clone()).await,
        "loc-build",
        "no query at all behaves the same"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_list_requires_admin(pool: PgPool) {
    // A non-admin session is forbidden; an absent token is unauthorized.
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let user = register_user(
        &pool,
        &format!("plain-{nonce}"),
        &format!("plain-{nonce}@example.com"),
        "pw",
    )
    .await
    .unwrap();
    let token = issue_session(&pool, user.id, Duration::from_secs(900))
        .await
        .unwrap()
        .token;

    let (status, _) = admin_get(pool.clone(), "/clients", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let app = loontail_catalog::admin_routes().with_state(state(pool));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

/// CB-15: the ten hand-written `if affected == 0` blocks became `found(affected, …)`.
/// `set_published` used to answer a bare `"not found"` for every kind of row, so the
/// admin SPA could not tell a missing client from a missing keyword. Pin the noun on
/// all three publishable kinds.
#[sqlx::test(migrations = "../../migrations")]
async fn publishing_a_missing_row_404s_and_names_the_entity(pool: PgPool) {
    let token = seed_admin_token(&pool).await;
    let missing = uuid::Uuid::new_v4();

    for (kind, entity) in [
        ("clients", "client"),
        ("keywords", "keyword"),
        ("servers", "server"),
    ] {
        let app = loontail_catalog::admin_routes().with_state(state(pool.clone()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/{kind}/{missing}/publish"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{kind} publish");
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body["error"]["message"],
            format!("{entity} not found"),
            "{kind} publish must name the entity"
        );
    }
}
