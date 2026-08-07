//! Socket-level integration tests for the two WebSocket endpoints join-via-relay is
//! built on: `/signaling` (per-user server push) and `/relay/{id}` (the host↔guest
//! byte tunnel). Unlike `network_api.rs`, these bind a real TCP listener and drive a
//! real WebSocket client, so the handshake, the authenticate-BEFORE-upgrade ordering
//! (failures must be plain HTTP status codes, not an accepted-then-closed socket) and
//! the frame plumbing are all exercised end to end.

mod support;

use std::time::Duration;

use axum::http::{StatusCode, Uri};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{ClientRequestBuilder, Error as WsError, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

use loontail_core::AppState;

use support::{authed, befriend, body_json, seed_user, TestUser};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How long a test waits for a frame or for a DB side effect to land before failing.
const DEADLINE: Duration = Duration::from_secs(5);

/// Poll an `await`-ing condition until it holds, or fail with `$label`. Needed for
/// state the handler writes *after* the handshake response has already been sent
/// (presence flips, relay status, player accounting, rendezvous parking) — polling
/// the real signal is what keeps these tests off `sleep`-shaped guesses.
macro_rules! eventually {
    ($label:expr, $cond:expr) => {{
        let deadline = tokio::time::Instant::now() + DEADLINE;
        while !$cond {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for: {}",
                $label
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }};
}

/// The router served over a real loopback socket, plus the `AppState` it shares with
/// the in-process `Router` used for REST setup. Sharing the state is what makes the
/// tests meaningful: the signaling hub and the relay rendezvous map are in-memory, so
/// a REST call that pushes an event must run against the SAME state as the socket.
struct Server {
    app: Router,
    state: AppState,
    base: String,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(pool: PgPool) -> Server {
    let state = support::state(pool);
    let app = loontail_network::routes().with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    let served = app.clone();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, served).await;
    });

    Server {
        app,
        state,
        base: format!("ws://127.0.0.1:{port}"),
        task,
    }
}

/// Open a WebSocket, sending `bearer` as an `Authorization` header when given.
async fn connect(url: &str, bearer: Option<&str>) -> Result<Socket, WsError> {
    let uri: Uri = url.parse().expect("ws url");
    let mut request = ClientRequestBuilder::new(uri);
    if let Some(token) = bearer {
        request = request.with_header("Authorization", format!("Bearer {token}"));
    }
    connect_async(request).await.map(|(socket, _)| socket)
}

/// The HTTP status a refused upgrade carried. Anything else — including a socket that
/// was accepted and then closed — is a failure: the handler contract is that auth runs
/// before the upgrade so clients see a real status code.
fn refusal_status(result: Result<Socket, WsError>) -> StatusCode {
    match result {
        Ok(_) => panic!("the upgrade was accepted, but it should have been refused"),
        Err(WsError::Http(resp)) => resp.status(),
        Err(other) => panic!("expected an HTTP refusal, got {other}"),
    }
}

/// Next payload frame, or a panic on timeout/close. Keepalives are skipped.
async fn next_frame(socket: &mut Socket) -> Message {
    tokio::time::timeout(DEADLINE, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(frame)) => return Some(frame),
                Some(Err(err)) => panic!("socket errored: {err}"),
                None => return None,
            }
        }
    })
    .await
    .expect("timed out waiting for a frame")
    .expect("socket closed while awaiting a frame")
}

async fn next_event(socket: &mut Socket) -> Value {
    match next_frame(socket).await {
        Message::Text(text) => serde_json::from_str(&text).expect("events are JSON"),
        other => panic!("expected a text event frame, got {other:?}"),
    }
}

async fn next_bytes(socket: &mut Socket) -> Vec<u8> {
    match next_frame(socket).await {
        Message::Binary(data) => data.to_vec(),
        other => panic!("expected a binary relay frame, got {other:?}"),
    }
}

async fn presence_status(pool: &PgPool, user_id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM presence WHERE user_id = $1::uuid")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("presence row")
}

async fn relay_status(pool: &PgPool, relay_session_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM relay_sessions WHERE id = $1")
        .bind(relay_session_id)
        .fetch_one(pool)
        .await
        .expect("relay session row")
}

async fn current_players(pool: &PgPool, world_id: &str) -> i32 {
    sqlx::query_scalar("SELECT current_players FROM world_sessions WHERE id = $1::uuid")
        .bind(world_id)
        .fetch_one(pool)
        .await
        .expect("world session row")
}

// --- /signaling ----------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn signaling_refuses_an_upgrade_with_no_bearer(pool: PgPool) {
    let server = serve(pool).await;

    let result = connect(&format!("{}/signaling", server.base), None).await;
    assert_eq!(
        refusal_status(result),
        StatusCode::UNAUTHORIZED,
        "an anonymous signaling upgrade is a 401, not an accepted socket"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn signaling_refuses_an_upgrade_with_an_unknown_bearer(pool: PgPool) {
    let server = serve(pool).await;

    let result = connect(
        &format!("{}/signaling", server.base),
        Some("not-a-real-session-token"),
    )
    .await;
    assert_eq!(refusal_status(result), StatusCode::UNAUTHORIZED);
}

/// The core signaling contract: an authenticated socket comes online, and an event
/// raised by somebody else's REST call is pushed to it as JSON.
#[sqlx::test(migrations = "../../migrations")]
async fn signaling_authenticates_then_receives_a_pushed_event(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let alice = seed_user(
        &pool,
        &server.app,
        "cccccccc-cccc-cccc-cccc-cccccccccc01",
        "wsalice",
    )
    .await;
    let bob = seed_user(
        &pool,
        &server.app,
        "cccccccc-cccc-cccc-cccc-cccccccccc02",
        "wsbob",
    )
    .await;

    let mut socket = connect(&format!("{}/signaling", server.base), Some(&alice.token))
        .await
        .expect("an authenticated signaling upgrade succeeds");

    // Connecting is what marks a user live, so the fan-out registry and the DB row
    // must both agree before an event can be routed anywhere.
    let alice_id: Uuid = alice.id.parse().unwrap();
    eventually!(
        "alice is registered in the signaling hub",
        server.state.realtime.signaling.is_online(alice_id)
    );
    eventually!(
        "alice's presence row flips online",
        presence_status(&pool, &alice.id).await == "online"
    );

    // Bob's REST call is the producer; alice's socket is the consumer.
    let resp = authed(
        &server.app,
        &bob,
        "POST",
        "/friends/requests",
        Some(json!({ "toUserId": alice.id })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let event = next_event(&mut socket).await;
    assert_eq!(
        event["type"], "friendRequest",
        "the pushed event is tagged, camelCased JSON"
    );
    assert_eq!(event["fromUser"]["id"], bob.id);
    assert_eq!(event["fromUser"]["username"], "wsbob");
    assert!(event["requestId"].as_str().is_some());

    // Closing the last connection takes the user offline again.
    socket.close(None).await.ok();
    drop(socket);
    eventually!(
        "alice's presence row flips offline",
        presence_status(&pool, &alice.id).await == "offline"
    );
}

/// A dropped socket must not poison the user's fan-out slot: the hub has to prune the
/// dead sender and route to the fresh one. This is the mod's reconnect path (its
/// client retries on a backoff after a transport drop).
#[sqlx::test(migrations = "../../migrations")]
async fn signaling_delivers_events_on_a_reconnected_socket(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let alice = seed_user(
        &pool,
        &server.app,
        "cccccccc-cccc-cccc-cccc-cccccccccc11",
        "rcalice",
    )
    .await;
    let bob = seed_user(
        &pool,
        &server.app,
        "cccccccc-cccc-cccc-cccc-cccccccccc12",
        "rcbob",
    )
    .await;
    let alice_id: Uuid = alice.id.parse().unwrap();

    let first = connect(&format!("{}/signaling", server.base), Some(&alice.token))
        .await
        .expect("first connect");
    eventually!(
        "alice is online",
        server.state.realtime.signaling.is_online(alice_id)
    );

    drop(first);
    eventually!(
        "the dropped connection is pruned from the hub",
        !server.state.realtime.signaling.is_online(alice_id)
    );

    let mut second = connect(&format!("{}/signaling", server.base), Some(&alice.token))
        .await
        .expect("reconnect with the same session token");
    eventually!(
        "alice is online again",
        server.state.realtime.signaling.is_online(alice_id)
    );

    authed(
        &server.app,
        &bob,
        "POST",
        "/friends/requests",
        Some(json!({ "toUserId": alice.id })),
    )
    .await;

    let event = next_event(&mut second).await;
    assert_eq!(
        event["type"], "friendRequest",
        "the event reaches the reconnected socket, not the dead one"
    );
}

// --- /relay/{id} ---------------------------------------------------------------

/// A host + guest pair, befriended, with the host `joinable` in an open world and the
/// guest already holding a join ticket for it.
struct JoinFixture {
    host: TestUser,
    guest: TestUser,
    world_id: String,
    relay_session_id: Uuid,
    ticket: String,
}

async fn join_fixture(pool: &PgPool, server: &Server, tag: &str) -> JoinFixture {
    let host = seed_user(
        pool,
        &server.app,
        &format!("dddddddd-dddd-dddd-dddd-dddddddd{tag}1"),
        &format!("rh{tag}"),
    )
    .await;
    let guest = seed_user(
        pool,
        &server.app,
        &format!("dddddddd-dddd-dddd-dddd-dddddddd{tag}2"),
        &format!("rg{tag}"),
    )
    .await;
    befriend(&server.app, &host, &guest).await;

    let world = body_json(
        authed(
            &server.app,
            &host,
            "POST",
            "/world-sessions",
            Some(json!({})),
        )
        .await,
    )
    .await;
    let world_id = world["id"].as_str().expect("world id").to_string();
    let resp = authed(
        &server.app,
        &host,
        "POST",
        "/presence/status",
        Some(json!({ "status": "joinable", "currentWorldSessionId": world_id })),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = authed(
        &server.app,
        &guest,
        "POST",
        &format!("/world-sessions/{world_id}/join-ticket"),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "the guest gets a ticket");
    let ticket = body_json(resp).await;

    JoinFixture {
        host,
        guest,
        world_id,
        relay_session_id: ticket["relaySessionId"]
            .as_str()
            .expect("relay session id")
            .parse()
            .expect("relay session id is a uuid"),
        ticket: ticket["ticket"].as_str().expect("ticket token").to_string(),
    }
}

fn relay_url(server: &Server, id: Uuid, role: &str) -> String {
    format!("{}/relay/{id}?role={role}", server.base)
}

/// The headline feature end to end: two sockets meet at the rendezvous, the guest is
/// admitted against the world's capacity, and raw bytes cross in both directions.
#[sqlx::test(migrations = "../../migrations")]
async fn relay_rendezvous_admits_a_guest_and_pipes_bytes_both_ways(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let fixture = join_fixture(&pool, &server, "aa").await;
    let relay_id = fixture.relay_session_id;

    // The host arrives first (in production it is cued by `guestArriving`) and parks.
    // Waiting for the park is what makes this deterministic: `take` runs before
    // `park`, so two simultaneous arrivals could both find the slot empty.
    let mut host_socket = connect(
        &relay_url(&server, relay_id, "host"),
        Some(&fixture.host.token),
    )
    .await
    .expect("the world host may open the relay");
    eventually!(
        "the host is parked at the rendezvous",
        server.state.realtime.relay.is_waiting(relay_id)
    );

    let mut guest_socket = connect(
        &relay_url(&server, relay_id, "guest"),
        Some(&fixture.ticket),
    )
    .await
    .expect("a valid ticket admits the guest");

    eventually!(
        "the pair is active",
        server.state.realtime.relay.active_pairings() == 1
    );
    eventually!(
        "the relay session is marked active",
        relay_status(&pool, relay_id).await == "active"
    );
    // Admission is the only place the player count advances, and it is the hard cap.
    eventually!(
        "the guest occupies a world player slot",
        current_players(&pool, &fixture.world_id).await == 1
    );

    // Guest -> host, then host -> guest: this is the TCP-over-WebSocket tunnel.
    guest_socket
        .send(Message::Binary(vec![0x10, 0x00, 0xC0, 0xDE].into()))
        .await
        .expect("guest sends");
    assert_eq!(
        next_bytes(&mut host_socket).await,
        vec![0x10, 0x00, 0xC0, 0xDE],
        "guest bytes reach the host verbatim"
    );

    host_socket
        .send(Message::Binary(vec![0x02, 0xFE, 0xED].into()))
        .await
        .expect("host sends");
    assert_eq!(
        next_bytes(&mut guest_socket).await,
        vec![0x02, 0xFE, 0xED],
        "host bytes reach the guest verbatim"
    );

    // The guest leaving tears the tunnel down and gives the slot back.
    guest_socket.close(None).await.ok();
    drop(guest_socket);
    eventually!(
        "the relay session is closed",
        relay_status(&pool, relay_id).await == "closed"
    );
    eventually!(
        "the player slot is released",
        current_players(&pool, &fixture.world_id).await == 0
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn relay_refuses_a_guest_with_a_bogus_ticket(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let fixture = join_fixture(&pool, &server, "bb").await;

    let result = connect(
        &relay_url(&server, fixture.relay_session_id, "guest"),
        Some("not-the-issued-ticket"),
    )
    .await;
    assert_eq!(refusal_status(result), StatusCode::UNAUTHORIZED);

    // The real ticket must survive a failed attempt — a bogus token cannot burn it.
    let consumed: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT jt.consumed_at FROM join_tickets jt \
         JOIN relay_sessions rs ON rs.join_ticket_id = jt.id WHERE rs.id = $1",
    )
    .bind(fixture.relay_session_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        consumed.is_none(),
        "a rejected attempt must not consume the single-use ticket"
    );
}

/// The guest's ticket is not a session token, and the session token is not a ticket:
/// each role validates its own credential, so swapping them must fail.
#[sqlx::test(migrations = "../../migrations")]
async fn relay_refuses_a_credential_used_for_the_wrong_role(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let fixture = join_fixture(&pool, &server, "cc").await;
    let relay_id = fixture.relay_session_id;

    // The guest is an authenticated user, but not this world's host.
    let result = connect(
        &relay_url(&server, relay_id, "host"),
        Some(&fixture.guest.token),
    )
    .await;
    assert_eq!(
        refusal_status(result),
        StatusCode::FORBIDDEN,
        "a non-host session on role=host is a 403"
    );

    // The host's session token is not a join ticket.
    let result = connect(
        &relay_url(&server, relay_id, "guest"),
        Some(&fixture.host.token),
    )
    .await;
    assert_eq!(
        refusal_status(result),
        StatusCode::UNAUTHORIZED,
        "a session token on role=guest is a 401"
    );

    let result = connect(
        &relay_url(&server, relay_id, "spectator"),
        Some(&fixture.ticket),
    )
    .await;
    assert_eq!(
        refusal_status(result),
        StatusCode::BAD_REQUEST,
        "role must be host or guest"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn relay_refuses_an_expired_ticket(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let fixture = join_fixture(&pool, &server, "dd").await;

    sqlx::query(
        "UPDATE join_tickets SET expires_at = now() - interval '1 second' \
         WHERE id = (SELECT join_ticket_id FROM relay_sessions WHERE id = $1)",
    )
    .bind(fixture.relay_session_id)
    .execute(&pool)
    .await
    .unwrap();

    let result = connect(
        &relay_url(&server, fixture.relay_session_id, "guest"),
        Some(&fixture.ticket),
    )
    .await;
    assert_eq!(
        refusal_status(result),
        StatusCode::UNAUTHORIZED,
        "an expired ticket is refused even though the token itself matches"
    );
}

/// The ticket is single-use and consumed at the upgrade, so a replay — a second
/// client presenting the same token, whether a leak or a double-click — is refused
/// while the first guest still holds the tunnel.
#[sqlx::test(migrations = "../../migrations")]
async fn relay_refuses_a_second_guest_replaying_the_same_ticket(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let fixture = join_fixture(&pool, &server, "ee").await;
    let relay_id = fixture.relay_session_id;

    let first = connect(
        &relay_url(&server, relay_id, "guest"),
        Some(&fixture.ticket),
    )
    .await
    .expect("the first guest is admitted");
    eventually!(
        "the first guest is parked at the rendezvous",
        server.state.realtime.relay.is_waiting(relay_id)
    );

    let result = connect(
        &relay_url(&server, relay_id, "guest"),
        Some(&fixture.ticket),
    )
    .await;
    assert_eq!(
        refusal_status(result),
        StatusCode::UNAUTHORIZED,
        "the ticket was consumed by the first guest"
    );

    // The refusal must not disturb the waiter that legitimately holds the slot.
    assert!(
        server.state.realtime.relay.is_waiting(relay_id),
        "the first guest keeps its rendezvous slot"
    );
    drop(first);
}

#[sqlx::test(migrations = "../../migrations")]
async fn relay_refuses_an_unknown_or_closed_session(pool: PgPool) {
    let server = serve(pool.clone()).await;
    let fixture = join_fixture(&pool, &server, "ff").await;

    let result = connect(
        &relay_url(&server, Uuid::new_v4(), "host"),
        Some(&fixture.host.token),
    )
    .await;
    assert_eq!(refusal_status(result), StatusCode::NOT_FOUND);

    sqlx::query("UPDATE relay_sessions SET status = 'closed' WHERE id = $1")
        .bind(fixture.relay_session_id)
        .execute(&pool)
        .await
        .unwrap();
    let result = connect(
        &relay_url(&server, fixture.relay_session_id, "host"),
        Some(&fixture.host.token),
    )
    .await;
    assert_eq!(
        refusal_status(result),
        StatusCode::CONFLICT,
        "a closed relay session cannot be reopened"
    );
}
