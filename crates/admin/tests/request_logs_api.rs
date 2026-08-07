//! Integration tests for the request-observability admin surface:
//! `/admin/logs/tail`, `/admin/analytics/requests`, `.../requests/summary`, and
//! `.../requests/timeseries`. Each test seeds `request_logs` rows directly and
//! drives the `Router<AppState>` through `tower::oneshot`, asserting the exact
//! camelCase shapes from the API contract and that every endpoint is
//! `AdminUser`-gated (401 without a session).

use std::time::Duration;

use axum::body::Body;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use loontail_admin::ensure_bootstrap_admin;
use loontail_core::auth::CSRF_COOKIE_NAME;
use loontail_core::config::Config;
use loontail_core::AppState;

const COOKIE_NAME: &str = "loontail_admin";

fn test_state(pool: PgPool) -> AppState {
    // why: only a placeholder when the var is absent — clobbering a real DATABASE_URL
    // trips sqlx::test's "DATABASE_URL changed at runtime" assertion mid-run.
    if std::env::var_os("DATABASE_URL").is_none() {
        std::env::set_var("DATABASE_URL", "postgres://unused");
    }
    let mut config = Config::from_env().unwrap();
    config.admin.cookie_name = COOKIE_NAME.to_string();
    config.admin.bootstrap_username = "rootadmin".to_string();
    config.admin.bootstrap_password = Some("rootpass123".to_string());
    config.admin.session_ttl = Duration::from_secs(900);
    AppState::new(pool, config)
}

fn app(state: AppState) -> Router {
    Router::new().nest_service("/admin", loontail_admin::routes().with_state(state))
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn cookie_from(resp: &axum::response::Response, name: &str) -> Option<String> {
    resp.headers().get_all(SET_COOKIE).iter().find_map(|hv| {
        let s = hv.to_str().ok()?;
        let first = s.split(';').next()?;
        let (k, v) = first.split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

struct Session {
    session_cookie: String,
}

async fn login(app: &Router) -> Session {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "username": "rootadmin", "password": "rootpass123" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // CSRF cookie is set on login; reads don't need it, but assert it exists.
    assert!(cookie_from(&resp, CSRF_COOKIE_NAME).is_some());
    let session_cookie = cookie_from(&resp, COOKIE_NAME).expect("session cookie set");
    Session { session_cookie }
}

impl Session {
    fn cookie(&self) -> String {
        format!("{COOKIE_NAME}={}", self.session_cookie)
    }
}

async fn get(app: &Router, uri: &str, session: Option<&Session>) -> axum::response::Response {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(s) = session {
        builder = builder.header(COOKIE, s.cookie());
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Seed one request_logs row at an offset from now. `ago_secs` shifts `ts` into
/// the past so window filters can be exercised.
#[allow(clippy::too_many_arguments)]
async fn seed(
    pool: &PgPool,
    method: &str,
    path: &str,
    status: i16,
    latency_ms: i32,
    auth_kind: &str,
    ago_secs: i64,
) {
    sqlx::query(
        "INSERT INTO request_logs (ts, method, path, status, latency_ms, auth_kind) \
         VALUES (now() - ($6 * interval '1 second'), $1, $2, $3, $4, $5)",
    )
    .bind(method)
    .bind(path)
    .bind(status)
    .bind(latency_ms)
    .bind(auth_kind)
    .bind(ago_secs as f64)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn endpoints_require_admin_session(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();
    let app = app(state);

    for uri in [
        "/admin/logs/tail",
        "/admin/analytics/requests",
        "/admin/analytics/requests/summary?window=24h",
        "/admin/analytics/requests/timeseries?window=24h&metric=requests",
    ] {
        let resp = get(&app, uri, None).await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{uri} must reject an unauthenticated caller"
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn requests_list_paginates_and_filters(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    // 3 ok GETs + 2 server errors + 1 POST to /api/auth/login.
    seed(&pool, "GET", "/api/me", 200, 5, "session", 10).await;
    seed(&pool, "GET", "/api/me", 200, 7, "session", 9).await;
    seed(&pool, "GET", "/api/friends", 200, 9, "session", 8).await;
    seed(&pool, "GET", "/api/me", 500, 3, "anon", 7).await;
    seed(&pool, "GET", "/api/me", 503, 4, "anon", 6).await;
    seed(&pool, "POST", "/api/auth/login", 200, 12, "anon", 5).await;

    let app = app(state);
    let session = login(&app).await;

    // Unfiltered: 6 rows, newest first, one page.
    let resp = get(&app, "/admin/analytics/requests", Some(&session)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["meta"]["total"], 6);
    assert_eq!(body["meta"]["page"], 1);
    assert_eq!(body["meta"]["pageSize"], 50);
    assert_eq!(body["meta"]["pageCount"], 1);
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 6);
    // Newest first: the POST /api/auth/login (ago 5s) is most recent.
    assert_eq!(data[0]["path"], "/api/auth/login");
    assert_eq!(data[0]["method"], "POST");
    // Row shape includes id + camelCase fields.
    assert!(data[0]["id"].is_number());
    assert!(data[0]["latencyMs"].is_number());
    assert_eq!(data[0]["authKind"], "anon");

    // Filter by method=GET → 5 rows.
    let resp = get(&app, "/admin/analytics/requests?method=GET", Some(&session)).await;
    let body = body_json(resp).await;
    assert_eq!(body["meta"]["total"], 5);

    // Filter by status=500 → 1 row.
    let resp = get(&app, "/admin/analytics/requests?status=500", Some(&session)).await;
    let body = body_json(resp).await;
    assert_eq!(body["meta"]["total"], 1);
    assert_eq!(body["data"][0]["status"], 500);

    // Filter by path prefix (/api/me) → 4 rows.
    let resp = get(
        &app,
        "/admin/analytics/requests?path=/api/me",
        Some(&session),
    )
    .await;
    let body = body_json(resp).await;
    assert_eq!(body["meta"]["total"], 4);
}

#[sqlx::test(migrations = "../../migrations")]
async fn persisted_row_maps_every_column_to_its_own_field(pool: PgPool) {
    // CB-2: the persisted page used to be read as an 11-element positional tuple, so
    // the type-identical `ip`/`user_agent` and `method`/`path` pairs could be swapped
    // by reordering the SELECT list without a compile error. Pin every column to its
    // wire field so a mapping swap fails here.
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO request_logs \
             (ts, method, path, status, latency_ms, auth_kind, ip, user_agent, bytes_out) \
         VALUES (now(), 'PATCH', '/api/distinctive-path', 418, 77, 'session', \
             '198.51.100.7', 'curl/8.0 probe', 4242)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = app(state);
    let session = login(&app).await;

    let resp = get(&app, "/admin/analytics/requests", Some(&session)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let row = body_json(resp).await["data"][0].clone();

    assert_eq!(row["method"], "PATCH");
    assert_eq!(row["path"], "/api/distinctive-path");
    assert_eq!(row["status"], 418);
    assert_eq!(row["latencyMs"], 77);
    assert_eq!(row["authKind"], "session");
    assert_eq!(row["ip"], "198.51.100.7");
    assert_eq!(row["userAgent"], "curl/8.0 probe");
    assert_eq!(row["bytesOut"], 4242);
    assert!(row["id"].is_number());
    assert!(row["userId"].is_null());
}

#[sqlx::test(migrations = "../../migrations")]
async fn requests_summary_shapes(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    // Within a 24h window: 4 x 2xx, 1 x 4xx, 2 x 5xx, mixed paths/latencies.
    seed(&pool, "GET", "/api/me", 200, 10, "session", 100).await;
    seed(&pool, "GET", "/api/me", 200, 20, "session", 200).await;
    seed(&pool, "GET", "/api/me", 304, 5, "session", 300).await;
    seed(&pool, "GET", "/api/friends", 200, 30, "session", 400).await;
    seed(&pool, "GET", "/api/friends", 404, 8, "anon", 500).await;
    seed(&pool, "GET", "/api/world", 500, 40, "anon", 600).await;
    seed(&pool, "GET", "/api/world", 503, 50, "anon", 700).await;
    // An old row outside the 24h window must be excluded.
    seed(&pool, "GET", "/api/old", 200, 1, "anon", 60 * 60 * 48).await;

    let app = app(state);
    let session = login(&app).await;

    let resp = get(
        &app,
        "/admin/analytics/requests/summary?window=24h",
        Some(&session),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;

    assert_eq!(body["window"], "24h");
    assert_eq!(body["totalRequests"], 7, "old row excluded by window");
    // error rate = server errors (>=500) / total = 2/7.
    let err_rate = body["errorRate"].as_f64().unwrap();
    assert!((err_rate - (2.0 / 7.0)).abs() < 1e-9, "got {err_rate}");
    // avg latency over the 7 in-window rows.
    let avg = body["avgLatencyMs"].as_f64().unwrap();
    let expected_avg = (10.0 + 20.0 + 5.0 + 30.0 + 8.0 + 40.0 + 50.0) / 7.0;
    assert!((avg - expected_avg).abs() < 1e-6, "got {avg}");

    // statusMix: 4 x 2xx (200,200,304? -> 304 is 3xx), recompute classes.
    // 200,200,200 -> 2xx(3); 304 -> 3xx(1); 404 -> 4xx(1); 500,503 -> 5xx(2).
    let mix = body["statusMix"].as_array().unwrap();
    let find = |class: &str| -> i64 {
        mix.iter()
            .find(|m| m["statusClass"] == class)
            .map(|m| m["count"].as_i64().unwrap())
            .unwrap_or(0)
    };
    assert_eq!(find("2xx"), 3);
    assert_eq!(find("3xx"), 1);
    assert_eq!(find("4xx"), 1);
    assert_eq!(find("5xx"), 2);

    // topPaths: /api/me (3) and /api/friends (2) and /api/world (2), ordered by count desc.
    let top = body["topPaths"].as_array().unwrap();
    assert_eq!(top[0]["path"], "/api/me");
    assert_eq!(top[0]["count"], 3);
    assert!(top[0]["avgLatencyMs"].is_number());
}

#[sqlx::test(migrations = "../../migrations")]
async fn requests_timeseries_metrics(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    // All within the last hour so they fall in one or two 24h buckets (by hour).
    seed(&pool, "GET", "/api/me", 200, 10, "session", 60).await;
    seed(&pool, "GET", "/api/me", 200, 30, "session", 120).await;
    seed(&pool, "GET", "/api/me", 500, 50, "anon", 180).await;

    let app = app(state);
    let session = login(&app).await;

    // requests metric: total value across buckets == 3.
    let resp = get(
        &app,
        "/admin/analytics/requests/timeseries?window=24h&metric=requests",
        Some(&session),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["window"], "24h");
    assert_eq!(body["metric"], "requests");
    let series = body["series"].as_array().unwrap();
    let total: f64 = series.iter().map(|p| p["value"].as_f64().unwrap()).sum();
    assert_eq!(total, 3.0);

    // errors metric: only the 500 counts.
    let resp = get(
        &app,
        "/admin/analytics/requests/timeseries?window=24h&metric=errors",
        Some(&session),
    )
    .await;
    let body = body_json(resp).await;
    assert_eq!(body["metric"], "errors");
    let series = body["series"].as_array().unwrap();
    let total: f64 = series.iter().map(|p| p["value"].as_f64().unwrap()).sum();
    assert_eq!(total, 1.0);

    // latency metric: average latency is positive and well-formed.
    let resp = get(
        &app,
        "/admin/analytics/requests/timeseries?window=24h&metric=latency",
        Some(&session),
    )
    .await;
    let body = body_json(resp).await;
    assert_eq!(body["metric"], "latency");
    let series = body["series"].as_array().unwrap();
    assert!(!series.is_empty());
    assert!(series.iter().all(|p| p["value"].as_f64().unwrap() >= 0.0));
}

#[sqlx::test(migrations = "../../migrations")]
async fn logs_tail_reflects_live_ring(pool: PgPool) {
    let state = test_state(pool.clone());
    ensure_bootstrap_admin(&pool, &state.config.admin)
        .await
        .unwrap();

    // The tail reads the in-memory ring, not the table. Push records directly into
    // the sink so the snapshot returns them newest-first.
    use loontail_core::request_log::RequestLog;
    for i in 0..3 {
        state.request_log.record(RequestLog {
            ts: chrono::Utc::now(),
            method: "GET".into(),
            path: format!("/api/p{i}"),
            status: 200,
            latency_ms: i,
            user_id: None,
            auth_kind: "anon".into(),
            ip: Some("1.2.3.4".into()),
            user_agent: Some("test-agent".into()),
            bytes_out: Some(42),
        });
    }

    let app = app(state);
    let session = login(&app).await;

    let resp = get(&app, "/admin/logs/tail?limit=2", Some(&session)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "limit honored");
    // Newest first: /api/p2 then /api/p1.
    assert_eq!(entries[0]["path"], "/api/p2");
    assert_eq!(entries[1]["path"], "/api/p1");
    // Tail rows carry no id (live ring) and the captured fields.
    assert!(entries[0].get("id").is_none(), "tail rows omit id");
    assert_eq!(entries[0]["ip"], "1.2.3.4");
    assert_eq!(entries[0]["userAgent"], "test-agent");
    assert_eq!(entries[0]["bytesOut"], 42);
    assert_eq!(entries[0]["authKind"], "anon");
}
