//! Integration tests for the network domain: bootstrap + current user,
//! search, the friend-request lifecycle (request/accept/decline), presence
//! (heartbeat/status/friends), world-session open/patch/close, and a join
//! happy path. Each test runs against an isolated Postgres via `#[sqlx::test]`
//! and drives the `Router<AppState>` through `tower::ServiceExt::oneshot` — no
//! real socket is opened. The socket-level suite lives in `websockets.rs`.

mod support;

use std::time::Duration;

use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use tower::ServiceExt;

use support::{
    app, authed, befriend, body_json, mint_session, post_bootstrap, seed_user, TestUser,
};

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_binds_identity_and_me_returns_user_and_presence(pool: PgPool) {
    let app = app(pool.clone());

    // Bootstrap now REQUIRES a session: an unauthenticated POST is rejected.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users/bootstrap")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "minecraftUuid": "11111111-1111-1111-1111-111111111111",
                        "username": "alice",
                        "minecraftVersion": "1.21.4",
                        "loader": "fabric"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // With a valid session, bootstrap binds the identity and marks presence online.
    let user = seed_user(&pool, &app, "11111111-1111-1111-1111-111111111111", "alice").await;

    // The session authenticates /me, which echoes the user + presence (bootstrap
    // initialised presence as online).
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
async fn bootstrap_rebinds_same_account_and_rejects_taken_minecraft_uuid(pool: PgPool) {
    let app = app(pool.clone());
    let mc_uuid = "22222222-2222-2222-2222-222222222222";

    // First account binds the identity.
    let bob = seed_user(&pool, &app, mc_uuid, "bob").await;

    // Re-bootstrapping the SAME account (same session) keeps one row and refreshes
    // last_seen — no new user is created and the binding is stable.
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_seen_at FROM users WHERE id = $1::uuid")
            .bind(&bob.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let resp = post_bootstrap(&app, &bob.token, mc_uuid, "bob").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["user"]["id"], bob.id);
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_seen_at FROM users WHERE id = $1::uuid")
            .bind(&bob.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(after >= before, "re-bootstrap refreshes last_seen");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE minecraft_uuid = $1")
        .bind(mc_uuid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "same account keeps one row for the minecraft uuid"
    );

    // A SECOND, DIFFERENT account that bootstraps with the ALREADY-BOUND
    // minecraft_uuid is a 409 Conflict (one Minecraft identity per account).
    let (_other_id, other_token) = mint_session(&pool, "bobby").await;
    let resp = post_bootstrap(&app, &other_token, mc_uuid, "bobby").await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_finds_other_users_by_username(pool: PgPool) {
    let app = app(pool.clone());
    let alice = seed_user(
        &pool,
        &app,
        "33333333-3333-3333-3333-333333333331",
        "searcher",
    )
    .await;
    seed_user(
        &pool,
        &app,
        "33333333-3333-3333-3333-333333333332",
        "findme",
    )
    .await;
    seed_user(
        &pool,
        &app,
        "33333333-3333-3333-3333-333333333333",
        "findme_too",
    )
    .await;

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
async fn search_treats_like_wildcards_literally(pool: PgPool) {
    let alice = seed_user(
        &pool,
        &app(pool.clone()),
        "55555555-5555-5555-5555-555555555551",
        "searcher",
    )
    .await;
    let app = app(pool.clone());
    // A username that genuinely contains a percent sign, plus a decoy that would
    // be swept in by an unescaped wildcard.
    seed_user(&pool, &app, "55555555-5555-5555-5555-555555555552", "ab%cd").await;
    seed_user(&pool, &app, "55555555-5555-5555-5555-555555555553", "abxcd").await;

    // The '%' must match literally: only "ab%cd" comes back, never "abxcd".
    let resp = authed(&app, &alice, "GET", "/users/search?q=b%25c", None).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let arr = body_json(resp).await;
    let arr = arr.as_array().unwrap();
    assert_eq!(arr.len(), 1, "literal % must not behave as a wildcard");
    assert_eq!(arr[0]["username"], "ab%cd");
}

#[sqlx::test(migrations = "../../migrations")]
async fn friend_request_accept_makes_both_friends(pool: PgPool) {
    let app = app(pool.clone());
    let a = seed_user(
        &pool,
        &app,
        "44444444-4444-4444-4444-444444444441",
        "afriend",
    )
    .await;
    let b = seed_user(
        &pool,
        &app,
        "44444444-4444-4444-4444-444444444442",
        "bfriend",
    )
    .await;

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
    let app = app(pool.clone());
    let a = seed_user(
        &pool,
        &app,
        "55555555-5555-5555-5555-555555555551",
        "decliner_a",
    )
    .await;
    let b = seed_user(
        &pool,
        &app,
        "55555555-5555-5555-5555-555555555552",
        "decliner_b",
    )
    .await;

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
    let app = app(pool.clone());
    let host = seed_user(&pool, &app, "66666666-6666-6666-6666-666666666661", "phost").await;
    let viewer = seed_user(
        &pool,
        &app,
        "66666666-6666-6666-6666-666666666662",
        "pviewer",
    )
    .await;

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
    let host = seed_user(&pool, &app, "77777777-7777-7777-7777-777777777771", "whost").await;

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
    let other = seed_user(
        &pool,
        &app,
        "77777777-7777-7777-7777-777777777772",
        "wother",
    )
    .await;
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

/// CB-6: `status`/`invitePolicy` are now the `WorldStatus`/`InvitePolicy` enums, but the
/// PATCH body is still parsed by hand — letting serde reject an unknown variant would
/// answer with a plain-text 422 and escape the `{error:{code,message}}` envelope every
/// other failure uses. Pin both the 400 and the envelope, and pin the wire spellings
/// the enums serialize to.
#[sqlx::test(migrations = "../../migrations")]
async fn world_patch_rejects_unknown_status_and_policy_in_the_error_envelope(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "77777777-7777-7777-7777-777777777791",
        "enumhost",
    )
    .await;
    let world_id = body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await)
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    for (field, value, expected) in [
        ("status", "bogus", "status must be open or closed"),
        (
            "invitePolicy",
            "everyone",
            "invitePolicy must be host_only or friends_of_friends",
        ),
    ] {
        let resp = authed(
            &app,
            &host,
            "PATCH",
            &format!("/world-sessions/{world_id}"),
            Some(json!({ field: value })),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{field}={value} must be a 400"
        );
        let body = body_json(resp).await;
        assert_eq!(body["error"]["message"], expected, "{field} message");
        assert!(
            body["error"]["code"].is_string(),
            "{field} rejection must use the error envelope"
        );
    }

    // The accepted spellings round-trip unchanged through the enums.
    let patched = body_json(
        authed(
            &app,
            &host,
            "PATCH",
            &format!("/world-sessions/{world_id}"),
            Some(json!({ "status": "open", "invitePolicy": "host_only" })),
        )
        .await,
    )
    .await;
    assert_eq!(patched["status"], "open");
    assert_eq!(patched["invitePolicy"], "host_only");
}

#[sqlx::test(migrations = "../../migrations")]
async fn join_ticket_happy_path_for_joinable_friend(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(&pool, &app, "88888888-8888-8888-8888-888888888881", "jhost").await;
    let guest = seed_user(
        &pool,
        &app,
        "88888888-8888-8888-8888-888888888882",
        "jguest",
    )
    .await;

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

    // TQ-02: the two fields the mod's authoritative compatibility gate reads
    // (LoontailNetworkClient.requestConnect -> checkCompat). They come from the
    // host's presence row via a LEFT JOIN, so a regression in that clause turns
    // "join a friend" into a permanent refusal toast with nothing else failing.
    assert_eq!(
        ticket["hostMinecraftVersion"], "1.21.4",
        "the gate reads the host's reported Minecraft version off the ticket"
    );
    assert_eq!(
        ticket["hostLoader"], "fabric",
        "the gate reads the host's reported mod loader off the ticket"
    );

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
    let stranger = seed_user(
        &pool,
        &app,
        "88888888-8888-8888-8888-888888888883",
        "jstranger",
    )
    .await;
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

/// TQ-02: the compat-gate fields come from a LEFT JOIN onto the host's presence
/// row, so a host that never reported a loader must yield `hostLoader: null` —
/// the mod turns that into `Loader.UNKNOWN` and fails the join closed. Pinning
/// the NULL deliberately, because the alternative (a stale value, or the join
/// silently dropping the ticket) would be a far worse failure.
#[sqlx::test(migrations = "../../migrations")]
async fn join_ticket_reports_null_host_loader_when_the_host_never_reported_one(pool: PgPool) {
    let app = app(pool.clone());

    // The host bootstraps WITHOUT `loader`, so presence.loader stays NULL.
    let (host_id, host_token) = mint_session(&pool, "nlhost").await;
    let host = TestUser {
        token: host_token,
        id: host_id.to_string(),
    };
    let resp = authed(
        &app,
        &host,
        "POST",
        "/users/bootstrap",
        Some(json!({
            "minecraftUuid": "77777777-7777-7777-7777-777777777771",
            "username": "nlhost",
            "minecraftVersion": "1.21.4"
        })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let loader: Option<String> =
        sqlx::query_scalar("SELECT loader FROM presence WHERE user_id = $1")
            .bind(host_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        loader.is_none(),
        "a bootstrap without `loader` must leave presence.loader NULL"
    );

    let guest = seed_user(
        &pool,
        &app,
        "77777777-7777-7777-7777-777777777772",
        "nlguest",
    )
    .await;

    befriend(&app, &host, &guest).await;

    let world =
        body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await).await;
    let world_id = world["id"].as_str().unwrap().to_string();
    authed(
        &app,
        &host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "joinable", "currentWorldSessionId": world_id })),
    )
    .await;

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
    assert_eq!(
        ticket["hostMinecraftVersion"], "1.21.4",
        "the version was reported, so it is still carried"
    );
    assert!(
        ticket["hostLoader"].is_null(),
        "an unreported host loader must reach the gate as null, not be omitted or faked"
    );
}

/// BUG-1: a credential-only friend (registered, never bootstrapped → NULL
/// `minecraft_uuid`) must not 500 the friends/requests/presence endpoints. The row
/// structs used to decode the nullable column into a non-`Option` `String`.
#[sqlx::test(migrations = "../../migrations")]
async fn credential_only_friend_does_not_500_friend_endpoints(pool: PgPool) {
    let app = app(pool.clone());

    // `caller` is a fully-bootstrapped user; `creduser` registers but NEVER
    // bootstraps, so its minecraft_uuid stays NULL (migration 0003 drops NOT NULL).
    let caller = seed_user(
        &pool,
        &app,
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1",
        "callerb1",
    )
    .await;
    let (cred_id, cred_token) = mint_session(&pool, "creduser").await;
    let cred = TestUser {
        token: cred_token,
        id: cred_id.to_string(),
    };

    // Sanity: the credential-only user genuinely has NULL minecraft_uuid.
    let mc: Option<String> = sqlx::query_scalar("SELECT minecraft_uuid FROM users WHERE id = $1")
        .bind(cred_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        mc.is_none(),
        "credential-only user must carry NULL minecraft_uuid"
    );

    // caller sends a request to the credential-only user. Building the request DTO
    // reads the credential-only user's NULL minecraft_uuid — this 500'd before the fix.
    let resp = authed(
        &app,
        &caller,
        "POST",
        "/friends/requests",
        Some(json!({ "toUserId": cred.id })),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "creating a request to a credential-only user must be 200, not 500"
    );
    let req = body_json(resp).await;
    let request_id = req["id"].as_str().unwrap().to_string();
    assert!(
        req["toUser"]["minecraftUuid"].is_null(),
        "credential-only user's minecraftUuid serializes as null"
    );

    // The credential-only user can list it as incoming (DTO assembly again).
    let incoming = authed(&app, &cred, "GET", "/friends/requests/incoming", None).await;
    assert_eq!(incoming.status(), StatusCode::OK);
    assert_eq!(body_json(incoming).await.as_array().unwrap().len(), 1);

    // caller lists it as outgoing.
    let outgoing = authed(&app, &caller, "GET", "/friends/requests/outgoing", None).await;
    assert_eq!(outgoing.status(), StatusCode::OK);

    // The credential-only user accepts → both become friends, then both list and
    // both presence endpoints must be 200 with the credential-only user present.
    let accept = authed(
        &app,
        &cred,
        "POST",
        &format!("/friends/requests/{request_id}/accept"),
        None,
    )
    .await;
    assert_eq!(accept.status(), StatusCode::OK);

    for (user, label) in [(&caller, "caller"), (&cred, "credential-only")] {
        let friends = authed(&app, user, "GET", "/friends", None).await;
        assert_eq!(friends.status(), StatusCode::OK, "GET /friends for {label}");
        let presence = authed(&app, user, "GET", "/presence/friends", None).await;
        assert_eq!(
            presence.status(),
            StatusCode::OK,
            "GET /presence/friends for {label}"
        );
    }

    // caller's friends list must include the credential-only user with null uuid.
    let caller_friends = body_json(authed(&app, &caller, "GET", "/friends", None).await).await;
    let arr = caller_friends.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["username"], "creduser");
    assert!(arr[0]["minecraftUuid"].is_null());
}

/// ARCH-1: the bootstrap hot path now emits a `bootstrap` analytics event off-thread,
/// so the admin dashboard timeseries (which reads `user_events`) is no longer empty in
/// production. The write is fire-and-forget, so poll briefly for the row, then run the
/// same aggregation the admin timeseries query uses and confirm it counts the event.
#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_emits_user_event_read_by_aggregation(pool: PgPool) {
    let app = app(pool.clone());
    let user = seed_user(
        &pool,
        &app,
        "cccccccc-cccc-cccc-cccc-ccccccccccc1",
        "analyticsuser",
    )
    .await;
    let user_id = uuid::Uuid::parse_str(&user.id).unwrap();

    // The event write is spawned (off the request), so poll until it lands.
    let mut events = 0i64;
    for _ in 0..40 {
        events = sqlx::query_scalar(
            "SELECT count(*) FROM user_events WHERE event_type = 'bootstrap' AND user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        if events > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        events, 1,
        "bootstrap must write exactly one user_events row"
    );

    // Mirror the admin timeseries aggregation (date_trunc bucketed count over the
    // indexed (event_type, created_at) range) to confirm the read side picks it up.
    let bucketed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_events \
         WHERE event_type = 'bootstrap' AND created_at > now() - interval '7 days'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bucketed, 1, "admin aggregation reads the bootstrap event");
}

/// BUG-3: a PATCH that transitions a world open→closed must run the same cleanup as
/// DELETE — close the world's active relay sessions, reset host presence to plain
/// online + null current_world_session_id, and zero current_players.
#[sqlx::test(migrations = "../../migrations")]
async fn patch_world_session_closed_runs_cleanup(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1",
        "patchhost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb2",
        "patchguest",
    )
    .await;

    // Host opens a world and enters it (presence points at the world).
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

    // Seed an ACTIVE relay session + an inflated player count directly, simulating a
    // guest in-world (the relay WS path can't be driven through oneshot).
    sqlx::query(
        "INSERT INTO relay_sessions (world_session_id, host_user_id, guest_user_id, status) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, 'active')",
    )
    .bind(&world_id)
    .bind(&host.id)
    .bind(&guest.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE world_sessions SET current_players = 3 WHERE id = $1::uuid")
        .bind(&world_id)
        .execute(&pool)
        .await
        .unwrap();

    // PATCH the world to status=closed.
    let resp = authed(
        &app,
        &host,
        "PATCH",
        &format!("/world-sessions/{world_id}"),
        Some(json!({ "status": "closed" })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let patched = body_json(resp).await;
    assert_eq!(patched["status"], "closed");
    assert_eq!(
        patched["currentPlayers"], 0,
        "current_players zeroed on close"
    );

    // Relay sessions for the world are all closed.
    let active_relays: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_sessions WHERE world_session_id = $1::uuid AND status = 'active'",
    )
    .bind(&world_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        active_relays, 0,
        "active relay sessions are closed on PATCH-close"
    );

    // Host presence is back to plain online with no current world.
    let (status, current): (String, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT status, current_world_session_id FROM presence WHERE user_id = $1::uuid",
    )
    .bind(&host.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "online", "host presence reset to online");
    assert!(current.is_none(), "host current_world_session_id nulled");

    // current_players is zeroed in the row, too.
    let players: i32 =
        sqlx::query_scalar("SELECT current_players FROM world_sessions WHERE id = $1::uuid")
            .bind(&world_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(players, 0);
}

/// TEST-1: friend-of-friend eligibility is re-validated at accept time, not just at
/// request time. A FoF requester legitimately creates a join-request (policy is FoF
/// AND they are a friend of an active member). If that trust link is then severed
/// before the host approves, accept must 403 — the host approving must not admit a
/// guest whose only path in has disappeared.
#[sqlx::test(migrations = "../../migrations")]
async fn fof_eligibility_revalidated_at_accept(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd1",
        "fofhost",
    )
    .await;
    let member = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd2",
        "fofmember",
    )
    .await;
    let requester = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd3",
        "fofreq",
    )
    .await;

    // member is a direct friend of host; requester is a friend of member only (the
    // friend-of-friend trust link), NOT a friend of host.
    befriend(&app, &host, &member).await;
    befriend(&app, &member, &requester).await;

    // Host opens an inWorld friends_of_friends world.
    let world =
        body_json(authed(&app, &host, "POST", "/world-sessions", Some(json!({}))).await).await;
    let world_id = world["id"].as_str().unwrap().to_string();
    authed(
        &app,
        &host,
        "PATCH",
        &format!("/world-sessions/{world_id}"),
        Some(json!({ "invitePolicy": "friends_of_friends" })),
    )
    .await;
    authed(
        &app,
        &host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "inWorld", "currentWorldSessionId": world_id })),
    )
    .await;

    // member is an ACTIVE guest in the world (the live FoF anchor for requester).
    sqlx::query(
        "INSERT INTO relay_sessions (world_session_id, host_user_id, guest_user_id, status) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, 'active')",
    )
    .bind(&world_id)
    .bind(&host.id)
    .bind(&member.id)
    .execute(&pool)
    .await
    .unwrap();

    // requester legitimately creates a join-request: not a host friend, but policy is
    // FoF and they are a friend of the active member.
    let resp = authed(
        &app,
        &requester,
        "POST",
        &format!("/world-sessions/{world_id}/join-requests"),
        None,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "FoF requester may create a join-request while the trust link is live"
    );
    let req_id = body_json(resp).await["id"].as_str().unwrap().to_string();

    // Sever the trust link: the anchoring member leaves (relay closed). The FoF path
    // for requester is now gone.
    sqlx::query("UPDATE relay_sessions SET status = 'closed' WHERE guest_user_id = $1::uuid")
        .bind(&member.id)
        .execute(&pool)
        .await
        .unwrap();

    // Host approves — but re-validation at accept must now reject the stale request.
    let resp = authed(
        &app,
        &host,
        "POST",
        &format!("/join-requests/{req_id}/accept"),
        None,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "accept must re-check FoF eligibility and reject once the link is severed"
    );

    // No relay session was minted for the rejected requester.
    let minted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_sessions WHERE guest_user_id = $1::uuid")
            .bind(&requester.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        minted, 0,
        "no ticket/relay issued for the rejected requester"
    );
}

/// TEST-1: the capacity gate rejects a join once the world is at max_players. The
/// atomic in-relay admission gate can't be driven through oneshot, but the
/// join-ticket handler's capacity precheck is — inflate current_players to the cap
/// and the next ticket request must 409, not over-admit.
#[sqlx::test(migrations = "../../migrations")]
async fn join_ticket_rejected_when_world_at_capacity(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1",
        "caphost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee2",
        "capguest",
    )
    .await;
    befriend(&app, &host, &guest).await;

    // Host opens a 1-slot world and is joinable.
    let world = body_json(
        authed(
            &app,
            &host,
            "POST",
            "/world-sessions",
            Some(json!({ "maxPlayers": 1 })),
        )
        .await,
    )
    .await;
    let world_id = world["id"].as_str().unwrap().to_string();
    authed(
        &app,
        &host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "joinable", "currentWorldSessionId": world_id })),
    )
    .await;

    // Fill the single slot.
    sqlx::query("UPDATE world_sessions SET current_players = 1 WHERE id = $1::uuid")
        .bind(&world_id)
        .execute(&pool)
        .await
        .unwrap();

    let resp = authed(
        &app,
        &guest,
        "POST",
        &format!("/world-sessions/{world_id}/join-ticket"),
        None,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "a full world must reject a new join-ticket with 409"
    );

    // No relay session was created for the rejected guest.
    let relays: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_sessions WHERE guest_user_id = $1::uuid")
            .bind(&guest.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(relays, 0);
}

/// Open a world for `host` and mark them `inWorld` in it, returning the world id.
async fn open_world_in_world(app: &Router, host: &TestUser) -> String {
    let world =
        body_json(authed(app, host, "POST", "/world-sessions", Some(json!({}))).await).await;
    let world_id = world["id"].as_str().unwrap().to_string();
    authed(
        app,
        host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "inWorld", "currentWorldSessionId": world_id })),
    )
    .await;
    world_id
}

/// Statuses of two concurrently issued POSTs, sorted so the pair reads
/// `[winner, loser]`.
async fn race_two_posts(app: &Router, user: &TestUser, uri: &str) -> Vec<StatusCode> {
    let (a, b) = tokio::join!(
        authed(app, user, "POST", uri, None),
        authed(app, user, "POST", uri, None),
    );
    let mut statuses = vec![a.status(), b.status()];
    statuses.sort_by_key(|s| s.as_u16());
    statuses
}

async fn count_rows(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

/// BE-02: two simultaneous accepts of the same invite must mint exactly one ticket
/// and one relay session — otherwise a scripted guest can occupy the whole world.
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_invite_accepts_issue_one_ticket(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff1",
        "racehost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff2",
        "raceguest",
    )
    .await;
    befriend(&app, &host, &guest).await;
    let world_id = open_world_in_world(&app, &host).await;

    let invite = body_json(
        authed(
            &app,
            &host,
            "POST",
            &format!("/world-sessions/{world_id}/invites"),
            Some(json!({ "inviteeUserId": guest.id })),
        )
        .await,
    )
    .await;
    let invite_id = invite["id"].as_str().unwrap().to_string();

    let statuses = race_two_posts(&app, &guest, &format!("/invites/{invite_id}/accept")).await;
    assert_eq!(
        statuses,
        vec![StatusCode::OK, StatusCode::CONFLICT],
        "exactly one accept wins, the other is a clean 409"
    );

    assert_eq!(count_rows(&pool, "join_tickets").await, 1);
    assert_eq!(count_rows(&pool, "relay_sessions").await, 1);
}

/// One accept = one pooled connection. The BE-02 transaction spans the whole
/// admission, so every read inside it must run on the transaction: a helper that
/// reaches back for `state.pool` makes the request need TWO connections, and then
/// `max_connections` simultaneous accepts each hold one while waiting for another —
/// all of them fail with `PoolTimedOut` -> 500 and starve every other DB-backed
/// route for the acquire timeout.
///
/// Pinned against a deliberately tiny pool (2 connections, 4 concurrent accepts of
/// distinct invites) because the 2-request race tests cannot exceed any real pool.
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_accepts_do_not_exhaust_a_small_pool(
    pool_opts: PgPoolOptions,
    connect_opts: PgConnectOptions,
) {
    let seed_pool = pool_opts
        .connect_with(connect_opts.clone())
        .await
        .expect("seed pool");
    let seed_app = app(seed_pool.clone());

    let host = seed_user(
        &seed_pool,
        &seed_app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeee00",
        "poolhost",
    )
    .await;
    let world_id = open_world_in_world(&seed_app, &host).await;

    let mut guests = Vec::new();
    let mut invite_ids = Vec::new();
    for n in 1..=4 {
        let guest = seed_user(
            &seed_pool,
            &seed_app,
            &format!("eeeeeeee-eeee-eeee-eeee-eeeeeeeeee0{n}"),
            &format!("poolguest{n}"),
        )
        .await;
        befriend(&seed_app, &host, &guest).await;
        let invite = body_json(
            authed(
                &seed_app,
                &host,
                "POST",
                &format!("/world-sessions/{world_id}/invites"),
                Some(json!({ "inviteeUserId": guest.id })),
            )
            .await,
        )
        .await;
        invite_ids.push(invite["id"].as_str().expect("invite id").to_string());
        guests.push(guest);
    }

    let tight_app = app(tight_pool(connect_opts).await);
    let uris: Vec<String> = invite_ids
        .iter()
        .map(|id| format!("/invites/{id}/accept"))
        .collect();
    let accept = |index: usize| authed(&tight_app, &guests[index], "POST", &uris[index], None);
    let (a, b, c, d) = tokio::join!(accept(0), accept(1), accept(2), accept(3));

    assert_all_ok([a, b, c, d]);
    assert_eq!(count_rows(&seed_pool, "join_tickets").await, 4);
}

/// [`concurrent_accepts_do_not_exhaust_a_small_pool`] for the join-request path, which
/// carries the identical transaction shape (and one more in-tx read,
/// `is_friend_of_active_member`).
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_join_request_accepts_do_not_exhaust_a_small_pool(
    pool_opts: PgPoolOptions,
    connect_opts: PgConnectOptions,
) {
    let seed_pool = pool_opts
        .connect_with(connect_opts.clone())
        .await
        .expect("seed pool");
    let seed_app = app(seed_pool.clone());

    let host = seed_user(
        &seed_pool,
        &seed_app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeee10",
        "jrpoolhost",
    )
    .await;
    let world_id = open_world_in_world(&seed_app, &host).await;

    let mut request_ids = Vec::new();
    for n in 1..=4 {
        let guest = seed_user(
            &seed_pool,
            &seed_app,
            &format!("eeeeeeee-eeee-eeee-eeee-eeeeeeeeee1{n}"),
            &format!("jrpoolguest{n}"),
        )
        .await;
        befriend(&seed_app, &host, &guest).await;
        let request = body_json(
            authed(
                &seed_app,
                &guest,
                "POST",
                &format!("/world-sessions/{world_id}/join-requests"),
                None,
            )
            .await,
        )
        .await;
        request_ids.push(request["id"].as_str().expect("request id").to_string());
    }

    let tight_app = app(tight_pool(connect_opts).await);
    let uris: Vec<String> = request_ids
        .iter()
        .map(|id| format!("/join-requests/{id}/accept"))
        .collect();
    let accept = |index: usize| authed(&tight_app, &host, "POST", &uris[index], None);
    let (a, b, c, d) = tokio::join!(accept(0), accept(1), accept(2), accept(3));

    assert_all_ok([a, b, c, d]);
    assert_eq!(count_rows(&seed_pool, "join_tickets").await, 4);
}

/// A pool smaller than the concurrency the test drives — the whole point. The short
/// acquire timeout keeps a regression a fast 500 rather than a multi-minute hang.
async fn tight_pool(connect_opts: PgConnectOptions) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(connect_opts)
        .await
        .expect("tight pool")
}

fn assert_all_ok(responses: [axum::response::Response; 4]) {
    for (index, resp) in responses.into_iter().enumerate() {
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "accept {index} must complete without needing a second pooled connection"
        );
    }
}

/// BE-02: the host double-clicking Accept on a join request must not mint two
/// tickets, two relay rows and two guest prompts.
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_join_request_accepts_issue_one_ticket(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff3",
        "racejrhost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff4",
        "racejrguest",
    )
    .await;
    befriend(&app, &host, &guest).await;
    let world_id = open_world_in_world(&app, &host).await;

    let request = body_json(
        authed(
            &app,
            &guest,
            "POST",
            &format!("/world-sessions/{world_id}/join-requests"),
            None,
        )
        .await,
    )
    .await;
    let request_id = request["id"].as_str().unwrap().to_string();

    let statuses =
        race_two_posts(&app, &host, &format!("/join-requests/{request_id}/accept")).await;
    assert_eq!(
        statuses,
        vec![StatusCode::OK, StatusCode::CONFLICT],
        "exactly one accept wins, the other is a clean 409"
    );

    assert_eq!(count_rows(&pool, "join_tickets").await, 1);
    assert_eq!(count_rows(&pool, "relay_sessions").await, 1);
}

/// BE-02: approving a friend-of-friend invite twice at once must release it — and
/// push `WorldInvite` to the invitee — exactly once.
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_invite_approves_emit_one_event(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff5",
        "racefofhost",
    )
    .await;
    let member = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff6",
        "racefofmember",
    )
    .await;
    let invitee = seed_user(
        &pool,
        &app,
        "ffffffff-ffff-ffff-ffff-fffffffffff7",
        "racefofinvitee",
    )
    .await;
    befriend(&app, &host, &member).await;
    befriend(&app, &member, &invitee).await;

    let world_id = open_world_in_world(&app, &host).await;
    authed(
        &app,
        &host,
        "PATCH",
        &format!("/world-sessions/{world_id}"),
        Some(json!({ "invitePolicy": "friends_of_friends" })),
    )
    .await;

    let invite = body_json(
        authed(
            &app,
            &member,
            "POST",
            &format!("/world-sessions/{world_id}/invites"),
            Some(json!({ "inviteeUserId": invitee.id })),
        )
        .await,
    )
    .await;
    assert_eq!(
        invite["status"], "pending_approval",
        "a friend-of-friend invite waits for the host"
    );
    let invite_id = invite["id"].as_str().unwrap().to_string();

    let statuses = race_two_posts(&app, &host, &format!("/invites/{invite_id}/approve")).await;
    assert_eq!(
        statuses,
        vec![StatusCode::OK, StatusCode::CONFLICT],
        "the second approval must 409 instead of re-pushing the invite"
    );
}

/// Every field of the `UserDto` shape a participant object must carry. The invite and
/// join-request lists were rebuilt on JOINs (BEQ-13 / COND-15); the old per-row
/// `SELECT * FROM users` assembly meant a dropped column would not have failed any
/// existing test, so assert each field explicitly.
fn assert_user_dto(actual: &Value, expected_id: &str, expected_username: &str, label: &str) {
    assert_eq!(actual["id"], expected_id, "{label}.id");
    assert_eq!(actual["username"], expected_username, "{label}.username");
    assert!(
        actual.get("minecraftUuid").is_some(),
        "{label}.minecraftUuid must be present (null is fine)"
    );
    assert_eq!(
        actual.as_object().unwrap().len(),
        3,
        "{label} carries exactly the three UserDto fields"
    );
}

/// BEQ-13 / COND-15: the invite lists are one JOIN now instead of 1 + 3N user reads.
/// Contract lock over a 2-row list: every participant object still carries all five
/// UserDto fields on all three sides.
#[sqlx::test(migrations = "../../migrations")]
async fn invite_lists_carry_full_participants_for_every_row(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee1",
        "dtohost",
    )
    .await;
    let a = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee2",
        "dtoguesta",
    )
    .await;
    let b = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee3",
        "dtoguestb",
    )
    .await;
    befriend(&app, &host, &a).await;
    befriend(&app, &host, &b).await;
    let world_id = open_world_in_world(&app, &host).await;

    for guest in [&a, &b] {
        let resp = authed(
            &app,
            &host,
            "POST",
            &format!("/world-sessions/{world_id}/invites"),
            Some(json!({ "inviteeUserId": guest.id })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Outgoing (the host's own two invites) is the 2-row list.
    let outgoing = body_json(authed(&app, &host, "GET", "/invites/outgoing", None).await).await;
    let rows = outgoing.as_array().expect("array");
    assert_eq!(rows.len(), 2, "both invites listed");
    for row in rows {
        assert_user_dto(&row["inviter"], &host.id, "dtohost", "inviter");
        assert_user_dto(&row["host"], &host.id, "dtohost", "host");
        let invitee_id = row["invitee"]["id"].as_str().unwrap();
        let expected = if invitee_id == a.id {
            "dtoguesta"
        } else {
            "dtoguestb"
        };
        assert_user_dto(&row["invitee"], invitee_id, expected, "invitee");
        assert_eq!(row["requiresHostApproval"], false);
    }

    // The single-row incoming view for one guest keeps the same shape.
    let incoming = body_json(authed(&app, &a, "GET", "/invites/incoming", None).await).await;
    let rows = incoming.as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_user_dto(&rows[0]["invitee"], &a.id, "dtoguesta", "incoming.invitee");
    assert_user_dto(&rows[0]["host"], &host.id, "dtohost", "incoming.host");
}

/// BEQ-13 / COND-15: same contract lock for the join-request list, where `incoming`
/// previously re-fetched the identical host row once per element.
#[sqlx::test(migrations = "../../migrations")]
async fn join_request_list_carries_full_participants(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee4",
        "jrdtohost",
    )
    .await;
    let a = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee5",
        "jrdtoa",
    )
    .await;
    let b = seed_user(
        &pool,
        &app,
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeee6",
        "jrdtob",
    )
    .await;
    befriend(&app, &host, &a).await;
    befriend(&app, &host, &b).await;
    let world_id = open_world_in_world(&app, &host).await;

    for guest in [&a, &b] {
        let resp = authed(
            &app,
            guest,
            "POST",
            &format!("/world-sessions/{world_id}/join-requests"),
            None,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let incoming =
        body_json(authed(&app, &host, "GET", "/join-requests/incoming", None).await).await;
    let rows = incoming.as_array().expect("array");
    assert_eq!(rows.len(), 2, "both requests listed");
    for row in rows {
        assert_user_dto(&row["host"], &host.id, "jrdtohost", "host");
        let requester_id = row["requester"]["id"].as_str().unwrap();
        let expected = if requester_id == a.id {
            "jrdtoa"
        } else {
            "jrdtob"
        };
        assert_user_dto(&row["requester"], requester_id, expected, "requester");
        assert_eq!(row["status"], "pending");
    }
}

/// BE-07: `join_requests` had no pending-uniqueness, so a friend could loop this
/// endpoint, inserting a row and firing a signaling push at the host every time.
#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_pending_join_request_is_409(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd1",
        "duphost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd2",
        "dupguest",
    )
    .await;
    befriend(&app, &host, &guest).await;
    let world_id = open_world_in_world(&app, &host).await;
    let uri = format!("/world-sessions/{world_id}/join-requests");

    let first = authed(&app, &guest, "POST", &uri, None).await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = authed(&app, &guest, "POST", &uri, None).await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a second pending request for the same world must be refused"
    );

    assert_eq!(
        count_rows(&pool, "join_requests").await,
        1,
        "no extra row, so no extra host prompt"
    );
}

/// The pending slot must stay reclaimable: once the first request lapses a new one is
/// allowed, or the partial unique index would block the pair forever behind a row no
/// list ever shows.
#[sqlx::test(migrations = "../../migrations")]
async fn expired_pending_join_request_allows_a_new_one(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd3",
        "exphost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "dddddddd-dddd-dddd-dddd-ddddddddddd4",
        "expguest",
    )
    .await;
    befriend(&app, &host, &guest).await;
    let world_id = open_world_in_world(&app, &host).await;
    let uri = format!("/world-sessions/{world_id}/join-requests");

    assert_eq!(
        authed(&app, &guest, "POST", &uri, None).await.status(),
        StatusCode::OK
    );
    // Backdate the pending row past its TTL, as the clock would.
    sqlx::query("UPDATE join_requests SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        authed(&app, &guest, "POST", &uri, None).await.status(),
        StatusCode::OK,
        "the lapsed request is expired first, freeing the unique slot"
    );

    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM join_requests ORDER BY created_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, vec!["expired".to_string(), "pending".to_string()]);
}

/// BE-07: nothing used to reclaim lapsed join/invite rows, spent tickets or
/// `user_events` — they grew for the life of the deployment. One cleanup pass must
/// expire the lapsed rows and delete history past the retention window.
#[sqlx::test(migrations = "../../migrations")]
async fn cleanup_expires_lapsed_rows_and_deletes_aged_history(pool: PgPool) {
    let app = app(pool.clone());
    let host = seed_user(
        &pool,
        &app,
        "cccccccc-cccc-cccc-cccc-ccccccccccc1",
        "cleanhost",
    )
    .await;
    let guest = seed_user(
        &pool,
        &app,
        "cccccccc-cccc-cccc-cccc-ccccccccccc2",
        "cleanguest",
    )
    .await;
    befriend(&app, &host, &guest).await;
    let world_id = open_world_in_world(&app, &host).await;

    // A pending join request and a pending invite, both then lapsed.
    authed(
        &app,
        &guest,
        "POST",
        &format!("/world-sessions/{world_id}/join-requests"),
        None,
    )
    .await;
    authed(
        &app,
        &host,
        "POST",
        &format!("/world-sessions/{world_id}/invites"),
        Some(json!({ "inviteeUserId": guest.id })),
    )
    .await;
    sqlx::query("UPDATE join_requests SET expires_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE world_invites SET expires_at = now() - interval '1 hour'")
        .execute(&pool)
        .await
        .unwrap();

    // Aged terminal history, a consumed ticket, and an aged analytics event — all past
    // the retention window.
    sqlx::query(
        "INSERT INTO join_requests \
             (requester_user_id, host_user_id, world_session_id, status, expires_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, 'declined', now(), now() - interval '40 days')",
    )
    .bind(&guest.id)
    .bind(&host.id)
    .bind(&world_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO join_tickets \
             (user_id, world_session_id, token_hash, expires_at, consumed_at, created_at) \
         VALUES ($1::uuid, $2::uuid, 'aged-hash', now(), now(), now() - interval '40 days')",
    )
    .bind(&guest.id)
    .bind(&world_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_events (user_id, event_type, created_at) \
         VALUES ($1::uuid, 'aged', now() - interval '40 days')",
    )
    .bind(&guest.id)
    .execute(&pool)
    .await
    .unwrap();
    let events_before = count_rows(&pool, "user_events").await;

    let counts = loontail_network::cleanup::cleanup_stale_join_state(&pool, 30)
        .await
        .expect("cleanup");

    assert_eq!(counts.expired_join_requests, 1, "lapsed request expired");
    assert_eq!(counts.expired_invites, 1, "lapsed invite expired");
    assert_eq!(
        counts.deleted_join_tickets, 1,
        "spent aged ticket reclaimed"
    );
    assert_eq!(
        counts.deleted_terminal_join_requests, 1,
        "aged terminal request deleted"
    );
    assert_eq!(counts.deleted_user_events, 1, "aged event deleted");

    // The rows this pass just expired are inside the window, so they survive it.
    assert_eq!(count_rows(&pool, "join_requests").await, 1);
    assert_eq!(count_rows(&pool, "world_invites").await, 1);
    assert_eq!(count_rows(&pool, "join_tickets").await, 0);
    assert_eq!(
        count_rows(&pool, "user_events").await,
        events_before - 1,
        "only the aged event went"
    );

    // Idempotent: a second pass has nothing left to do.
    let again = loontail_network::cleanup::cleanup_stale_join_state(&pool, 30)
        .await
        .expect("cleanup");
    assert_eq!(again.total(), 0);
}
