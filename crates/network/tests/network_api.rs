//! Integration tests for the network domain: bootstrap + current user,
//! search, the friend-request lifecycle (request/accept/decline), presence
//! (heartbeat/status/friends), world-session open/patch/close, and a join
//! happy path. Each test runs against an isolated Postgres via `#[sqlx::test]`
//! and drives the `Router<AppState>` through `tower::ServiceExt::oneshot` — no
//! real socket is opened.

use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use loontail_core::config::Config;
use loontail_core::AppState;

/// Build the network router with state wired to the injected test pool. The pool
/// comes from `#[sqlx::test]`, so the `DATABASE_URL` placeholder is never used.
fn app(pool: PgPool) -> Router {
    std::env::set_var("DATABASE_URL", "postgres://unused");
    let mut config = Config::from_env().unwrap();
    // A generous heartbeat window so freshly-seeded presence rows read as live
    // across the whole test, independent of wall-clock timing.
    config.heartbeat_timeout = std::time::Duration::from_secs(3600);
    let state = AppState::new(pool, config);
    loontail_network::routes().with_state(state)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// A bootstrapped network user: its issued bearer token plus the public DTO the
/// server returned, so tests can reference its id without a second round-trip.
struct TestUser {
    token: String,
    id: String,
}

/// `POST /users/bootstrap` for a synthetic mod user and assert the shape, then
/// return the token + identity for follow-up authenticated calls.
async fn bootstrap(app: &Router, minecraft_uuid: &str, username: &str) -> TestUser {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users/bootstrap")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "minecraftUuid": minecraft_uuid,
                        "username": username,
                        "minecraftVersion": "1.21.4",
                        "loader": "fabric"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "bootstrap should succeed");
    let body = body_json(resp).await;
    let token = body["token"].as_str().expect("token issued").to_string();
    assert!(
        body["expiresAt"].as_str().is_some(),
        "bootstrap returns an expiry"
    );
    let user = &body["user"];
    assert_eq!(user["username"], username);
    assert_eq!(user["minecraftUuid"], minecraft_uuid);
    TestUser {
        token,
        id: user["id"].as_str().expect("user id").to_string(),
    }
}

/// Issue an authenticated request as `user`, returning the raw response.
async fn authed(
    app: &Router,
    user: &TestUser,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {}", user.token));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_issues_token_and_me_returns_user_and_presence(pool: PgPool) {
    let app = app(pool);
    let user = bootstrap(&app, "11111111-1111-1111-1111-111111111111", "alice").await;

    // The issued token authenticates /me, which echoes the user + presence
    // (bootstrap initialised presence as online).
    let resp = authed(&app, &user, "GET", "/me", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let me = body_json(resp).await;
    assert_eq!(me["user"]["username"], "alice");
    assert_eq!(me["user"]["id"], user.id);
    assert_eq!(me["status"], "online");

    // A missing / bogus bearer token is rejected.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/me")
                .header(AUTHORIZATION, "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_is_idempotent_per_minecraft_uuid(pool: PgPool) {
    let app = app(pool.clone());
    let first = bootstrap(&app, "22222222-2222-2222-2222-222222222222", "bob").await;
    // Same minecraft_uuid, renamed: upsert keeps one row, updates the username.
    let second = bootstrap(&app, "22222222-2222-2222-2222-222222222222", "bobby").await;
    assert_eq!(first.id, second.id, "same minecraft uuid maps to one user");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let name: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1::uuid")
        .bind(&first.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "bobby");
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_finds_other_users_by_username(pool: PgPool) {
    let app = app(pool);
    let alice = bootstrap(&app, "33333333-3333-3333-3333-333333333331", "searcher").await;
    bootstrap(&app, "33333333-3333-3333-3333-333333333332", "findme").await;
    bootstrap(&app, "33333333-3333-3333-3333-333333333333", "findme_too").await;

    // A substring match returns both "findme" users but never the caller.
    let resp = authed(&app, &alice, "GET", "/users/search?q=findme", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let results = body_json(resp).await;
    let arr = results.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    for r in arr {
        assert_ne!(r["id"], alice.id, "search excludes the caller");
        assert_eq!(r["isFriend"], false);
        assert_eq!(r["hasIncomingRequest"], false);
        assert_eq!(r["hasOutgoingRequest"], false);
    }

    // A too-short query is a 400 (min length is 2).
    let resp = authed(&app, &alice, "GET", "/users/search?q=f", None).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "../../migrations")]
async fn friend_request_accept_makes_both_friends(pool: PgPool) {
    let app = app(pool);
    let a = bootstrap(&app, "44444444-4444-4444-4444-444444444441", "afriend").await;
    let b = bootstrap(&app, "44444444-4444-4444-4444-444444444442", "bfriend").await;

    // A sends B a friend request.
    let resp = authed(
        &app,
        &a,
        "POST",
        "/friends/requests",
        Some(json!({ "toUserId": b.id })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let req = body_json(resp).await;
    let request_id = req["id"].as_str().unwrap().to_string();
    assert_eq!(req["status"], "pending");
    assert_eq!(req["fromUser"]["username"], "afriend");
    assert_eq!(req["toUser"]["username"], "bfriend");

    // B sees it as incoming; A sees it as outgoing.
    let incoming =
        body_json(authed(&app, &b, "GET", "/friends/requests/incoming", None).await).await;
    assert_eq!(incoming.as_array().unwrap().len(), 1);
    assert_eq!(incoming[0]["id"], request_id);
    let outgoing =
        body_json(authed(&app, &a, "GET", "/friends/requests/outgoing", None).await).await;
    assert_eq!(outgoing.as_array().unwrap().len(), 1);

    // B accepts. Both now list each other under /friends.
    let resp = authed(
        &app,
        &b,
        "POST",
        &format!("/friends/requests/{request_id}/accept"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "accepted");

    let a_friends = body_json(authed(&app, &a, "GET", "/friends", None).await).await;
    assert_eq!(a_friends.as_array().unwrap().len(), 1);
    assert_eq!(a_friends[0]["username"], "bfriend");
    let b_friends = body_json(authed(&app, &b, "GET", "/friends", None).await).await;
    assert_eq!(b_friends.as_array().unwrap().len(), 1);
    assert_eq!(b_friends[0]["username"], "afriend");

    // The incoming list is now empty (the request is no longer pending).
    let incoming =
        body_json(authed(&app, &b, "GET", "/friends/requests/incoming", None).await).await;
    assert_eq!(incoming.as_array().unwrap().len(), 0);

    // search now reflects the friendship.
    let results = body_json(authed(&app, &a, "GET", "/users/search?q=bfriend", None).await).await;
    assert_eq!(results[0]["isFriend"], true);
}

#[sqlx::test(migrations = "../../migrations")]
async fn friend_request_decline_leaves_no_friendship(pool: PgPool) {
    let app = app(pool);
    let a = bootstrap(&app, "55555555-5555-5555-5555-555555555551", "decliner_a").await;
    let b = bootstrap(&app, "55555555-5555-5555-5555-555555555552", "decliner_b").await;

    let req = body_json(
        authed(
            &app,
            &a,
            "POST",
            "/friends/requests",
            Some(json!({ "toUserId": b.id })),
        )
        .await,
    )
    .await;
    let request_id = req["id"].as_str().unwrap().to_string();

    let resp = authed(
        &app,
        &b,
        "POST",
        &format!("/friends/requests/{request_id}/decline"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "declined");

    // Neither side has a friend.
    let a_friends = body_json(authed(&app, &a, "GET", "/friends", None).await).await;
    assert_eq!(a_friends.as_array().unwrap().len(), 0);
    let b_friends = body_json(authed(&app, &b, "GET", "/friends", None).await).await;
    assert_eq!(b_friends.as_array().unwrap().len(), 0);

    // A non-recipient cannot decline someone else's request: re-send (fresh
    // pending) then have A (the sender, not the recipient) try to decline.
    let req = body_json(
        authed(
            &app,
            &a,
            "POST",
            "/friends/requests",
            Some(json!({ "toUserId": b.id })),
        )
        .await,
    )
    .await;
    let request_id = req["id"].as_str().unwrap();
    let resp = authed(
        &app,
        &a,
        "POST",
        &format!("/friends/requests/{request_id}/decline"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../migrations")]
async fn presence_heartbeat_status_and_friends_view(pool: PgPool) {
    let app = app(pool);
    let host = bootstrap(&app, "66666666-6666-6666-6666-666666666661", "phost").await;
    let viewer = bootstrap(&app, "66666666-6666-6666-6666-666666666662", "pviewer").await;

    // Make them friends so the viewer can observe the host's presence.
    let req = body_json(
        authed(
            &app,
            &viewer,
            "POST",
            "/friends/requests",
            Some(json!({ "toUserId": host.id })),
        )
        .await,
    )
    .await;
    let request_id = req["id"].as_str().unwrap();
    let resp = authed(
        &app,
        &host,
        "POST",
        &format!("/friends/requests/{request_id}/accept"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Heartbeat keeps the host online.
    let hb = body_json(authed(&app, &host, "POST", "/presence/heartbeat", None).await).await;
    assert_eq!(hb["status"], "online");

    // The host opens a world and declares itself inWorld pointing at it.
    let world =
        body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await).await;
    let world_id = world["id"].as_str().unwrap().to_string();

    let resp = authed(
        &app,
        &host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "inWorld", "currentWorldSessionId": world_id })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["status"], "inWorld");

    // The viewer's friends-presence reflects the host as inWorld at that world.
    let friends = body_json(authed(&app, &viewer, "GET", "/presence/friends", None).await).await;
    let arr = friends.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["username"], "phost");
    assert_eq!(arr[0]["status"], "inWorld");
    assert_eq!(arr[0]["currentWorldSessionId"], world_id);

    // Setting inWorld with a world the caller does not own is rejected.
    let resp = authed(
        &app,
        &viewer,
        "POST",
        "/presence/status",
        Some(json!({ "status": "inWorld", "currentWorldSessionId": world_id })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "../../migrations")]
async fn world_session_open_is_idempotent_patch_and_close(pool: PgPool) {
    let app = app(pool.clone());
    let host = bootstrap(&app, "77777777-7777-7777-7777-777777777771", "whost").await;

    // Open: idempotent — a second open returns the same row (one open per host).
    let first = body_json(
        authed(
            &app,
            &host,
            "POST",
            "/world-sessions",
            Some(json!({ "maxPlayers": 4 })),
        )
        .await,
    )
    .await;
    let world_id = first["id"].as_str().unwrap().to_string();
    assert_eq!(first["status"], "open");
    assert_eq!(first["maxPlayers"], 4);

    let second =
        body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await).await;
    assert_eq!(second["id"], world_id, "one open world per host");

    let open_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM world_sessions WHERE status = 'open'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(open_count, 1);

    // Patch: raise capacity and switch invite policy.
    let resp = authed(
        &app,
        &host,
        "PATCH",
        &format!("/world-sessions/{world_id}"),
        Some(json!({ "maxPlayers": 5, "invitePolicy": "friends_of_friends" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let patched = body_json(resp).await;
    assert_eq!(patched["maxPlayers"], 5);
    assert_eq!(patched["invitePolicy"], "friends_of_friends");

    // A non-host cannot patch the world.
    let other = bootstrap(&app, "77777777-7777-7777-7777-777777777772", "wother").await;
    let resp = authed(
        &app,
        &other,
        "PATCH",
        &format!("/world-sessions/{world_id}"),
        Some(json!({ "maxPlayers": 2 })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Close: the world is marked closed and a re-open issues a fresh session.
    let resp = authed(
        &app,
        &host,
        "DELETE",
        &format!("/world-sessions/{world_id}"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["closed"], true);

    let status: String =
        sqlx::query_scalar("SELECT status FROM world_sessions WHERE id = $1::uuid")
            .bind(&world_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "closed");

    let reopened =
        body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await).await;
    assert_ne!(
        reopened["id"], world_id,
        "closing frees the slot for a new world"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn join_ticket_happy_path_for_joinable_friend(pool: PgPool) {
    let app = app(pool.clone());
    let host = bootstrap(&app, "88888888-8888-8888-8888-888888888881", "jhost").await;
    let guest = bootstrap(&app, "88888888-8888-8888-8888-888888888882", "jguest").await;

    // Befriend so the guest is allowed to join.
    let req = body_json(
        authed(
            &app,
            &host,
            "POST",
            "/friends/requests",
            Some(json!({ "toUserId": guest.id })),
        )
        .await,
    )
    .await;
    let request_id = req["id"].as_str().unwrap();
    authed(
        &app,
        &guest,
        "POST",
        &format!("/friends/requests/{request_id}/accept"),
        None,
    )
    .await;

    // Host opens a world and is joinable (free join, no approval needed).
    let world =
        body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await).await;
    let world_id = world["id"].as_str().unwrap().to_string();
    let resp = authed(
        &app,
        &host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "joinable", "currentWorldSessionId": world_id })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Guest requests a join ticket and gets one immediately.
    let resp = authed(
        &app,
        &guest,
        "POST",
        &format!("/world-sessions/{world_id}/join-ticket"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ticket = body_json(resp).await;
    assert!(
        ticket["ticket"].as_str().unwrap().len() >= 32,
        "an opaque ticket token is returned"
    );
    assert_eq!(ticket["worldSessionId"], world_id);
    assert_eq!(ticket["hostUserId"], host.id);
    assert!(ticket["relaySessionId"].as_str().is_some());

    // A pending relay session was created for this guest+host pairing.
    let relay_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_sessions WHERE world_session_id = $1::uuid \
         AND host_user_id = $2::uuid AND guest_user_id = $3::uuid AND status = 'pending'",
    )
    .bind(&world_id)
    .bind(&host.id)
    .bind(&guest.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(relay_count, 1);

    // A non-friend stranger is forbidden from grabbing a ticket.
    let stranger = bootstrap(&app, "88888888-8888-8888-8888-888888888883", "jstranger").await;
    let resp = authed(
        &app,
        &stranger,
        "POST",
        &format!("/world-sessions/{world_id}/join-ticket"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // The host cannot join its own world.
    let resp = authed(
        &app,
        &host,
        "POST",
        &format!("/world-sessions/{world_id}/join-ticket"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
