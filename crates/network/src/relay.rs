//! Relay / tunnel MVP.
//!
//! Architecture: each guest joining a world gets its own `relay_session`
//! (a rendezvous pair identified by `relay_session_id`). The host opens one
//! relay WebSocket per arriving guest (cued by the `guestArriving` signaling
//! event) and the guest opens one too; the hub pairs them by id and pipes raw
//! bytes both ways. One pair per guest scales naturally to `max_players`.
//!
//! The mod side bridges each WebSocket to a local TCP socket (host: the LAN
//! world; guest: a local listener Minecraft connects to), turning this into a
//! TCP-over-WebSocket tunnel.
//!
//! The rendezvous data structure (`RelayHub`) lives in `core::realtime`; this
//! module owns only the axum WS handler that drives it.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use loontail_core::auth::{self, hash_token};
use loontail_core::error::{AppError, AppResult};
use loontail_core::realtime::PendingPeer;
use loontail_core::AppState;
use loontail_core::Metrics;

use crate::presence;

/// How long the first party waits at the rendezvous for its peer.
const RENDEZVOUS_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(sqlx::FromRow)]
struct RelaySessionRow {
    world_session_id: Uuid,
    host_user_id: Uuid,
    guest_user_id: Option<Uuid>,
    join_ticket_id: Option<Uuid>,
    status: String,
}

#[derive(Debug, Deserialize)]
pub struct RelayQuery {
    /// Optional `?token=` fallback; the Bearer header is preferred.
    pub token: Option<String>,
    pub role: String,
}

/// `GET /relay/:relaySessionId?role=host|guest` — upgrade to the relay tunnel
/// WebSocket. Auth comes from the `Authorization: Bearer` header (like REST),
/// with a `?token=` fallback; `role` stays in the query.
pub async fn relay_ws(
    State(state): State<AppState>,
    Path(relay_session_id): Path<Uuid>,
    Query(query): Query<RelayQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    let token = auth::bearer_token_from_headers(&headers)
        .or(query.token)
        .ok_or(AppError::Unauthorized)?;

    let relay = sqlx::query_as::<_, RelaySessionRow>(
        r#"
        SELECT world_session_id, host_user_id, guest_user_id, join_ticket_id, status
        FROM relay_sessions WHERE id = $1
        "#,
    )
    .bind(relay_session_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("relay session not found".into()))?;

    if relay.status == "closed" {
        return Err(AppError::Conflict("relay session is closed".into()));
    }

    match query.role.as_str() {
        "host" => {
            let user = auth::user_from_token(&state.pool, &token).await?;
            if user.id != relay.host_user_id {
                return Err(AppError::Forbidden);
            }
        }
        "guest" => {
            validate_guest_ticket(&state, &relay, &token).await?;
        }
        _ => return Err(AppError::BadRequest("role must be host or guest".into())),
    }

    let world_session_id = relay.world_session_id;
    let guest_user_id = relay.guest_user_id;
    Ok(ws.on_upgrade(move |socket| {
        handle_relay(
            socket,
            state,
            relay_session_id,
            world_session_id,
            guest_user_id,
        )
    }))
}

async fn validate_guest_ticket(
    state: &AppState,
    relay: &RelaySessionRow,
    token: &str,
) -> AppResult<()> {
    let ticket_id = relay
        .join_ticket_id
        .ok_or_else(|| AppError::Conflict("relay session has no ticket".into()))?;
    let token_hash = hash_token(token);

    let valid: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM join_tickets
            WHERE id = $1
              AND token_hash = $2
              AND user_id = $3
              AND consumed_at IS NULL
              AND expires_at > now()
        )
        "#,
    )
    .bind(ticket_id)
    .bind(&token_hash)
    .bind(relay.guest_user_id)
    .fetch_one(&state.pool)
    .await?;

    if !valid {
        return Err(AppError::Unauthorized);
    }

    // Single-use: consume the ticket now.
    sqlx::query(
        "UPDATE join_tickets SET consumed_at = now() WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(ticket_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}

async fn handle_relay(
    socket: WebSocket,
    state: AppState,
    relay_session_id: Uuid,
    world_session_id: Uuid,
    guest_user_id: Option<Uuid>,
) {
    // Are we the second party? If so, a peer is already waiting.
    let pending = state.realtime.relay.take(relay_session_id);

    match pending {
        Some(peer) => {
            // Authoritative capacity gate at the real admission moment. The
            // per-path pre-checks (join-ticket / join-request / invite accept)
            // read `current_players`, which only advances HERE, so two guests can
            // both pass a pre-check before either is counted. This atomic
            // conditional increment is the only place that cannot be raced: it
            // succeeds for a guest only while a slot is actually free.
            if !try_admit_player(&state, world_session_id).await {
                mark_relay_closed(&state, relay_session_id).await;
                let _ = peer.done.send(());
                return; // world full: both sockets drop here, refusing the guest
            }

            state.realtime.relay.incr_active();
            Metrics::incr(&state.metrics.relay_sessions_opened);
            mark_relay_active(&state, relay_session_id).await;
            // The guest is now an active member: their friends should see them
            // in-world (for friend-of-friends worlds) in real time.
            if let Some(guest) = guest_user_id {
                let _ = presence::broadcast_presence(&state, guest).await;
            }

            pipe(peer.socket, socket, &state).await;

            let _ = peer.done.send(());
            state.realtime.relay.decr_active();
            release_player(&state, world_session_id).await;
            mark_relay_closed(&state, relay_session_id).await;
            // The guest left: refresh their friends' view back to plain online.
            if let Some(guest) = guest_user_id {
                let _ = presence::broadcast_presence(&state, guest).await;
            }
        }
        None => {
            // First party: park our socket and wait for the peer to pipe.
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            state.realtime.relay.park(
                relay_session_id,
                PendingPeer {
                    socket,
                    done: done_tx,
                },
            );

            let _ = tokio::time::timeout(RENDEZVOUS_TIMEOUT, done_rx).await;

            // Clean up if we timed out before a peer arrived.
            let _ = state.realtime.relay.take(relay_session_id);
        }
    }
}

/// Atomically reserve a player slot for a world. Returns `true` only if the
/// world is still open and had a free slot (now taken). A single SQL statement,
/// so concurrent callers cannot both pass the `current_players < max_players`
/// guard — this is what makes the cap a hard limit.
async fn try_admit_player(state: &AppState, world_session_id: Uuid) -> bool {
    match sqlx::query(
        r#"
        UPDATE world_sessions
        SET current_players = current_players + 1
        WHERE id = $1 AND status = 'open' AND current_players < max_players
        "#,
    )
    .bind(world_session_id)
    .execute(&state.pool)
    .await
    {
        Ok(result) => result.rows_affected() > 0,
        Err(err) => {
            tracing::warn!(error = %err, "failed to reserve world player slot");
            false
        }
    }
}

/// Release a player slot previously reserved by `try_admit_player` (guest left).
async fn release_player(state: &AppState, world_session_id: Uuid) {
    if let Err(err) = sqlx::query(
        "UPDATE world_sessions SET current_players = GREATEST(current_players - 1, 0) WHERE id = $1",
    )
    .bind(world_session_id)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(error = %err, "failed to release world player slot");
    }
}

async fn mark_relay_active(state: &AppState, relay_session_id: Uuid) {
    if let Err(err) = sqlx::query(
        "UPDATE relay_sessions SET status = 'active' WHERE id = $1 AND status <> 'closed'",
    )
    .bind(relay_session_id)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(error = %err, "failed to mark relay session active");
    }
}

async fn mark_relay_closed(state: &AppState, relay_session_id: Uuid) {
    if let Err(err) =
        sqlx::query("UPDATE relay_sessions SET status = 'closed', closed_at = now() WHERE id = $1")
            .bind(relay_session_id)
            .execute(&state.pool)
            .await
    {
        tracing::warn!(error = %err, "failed to mark relay session closed");
    }
}

/// Pipe raw frames bidirectionally until either side closes.
async fn pipe(host: WebSocket, guest: WebSocket, state: &AppState) {
    let (mut host_sink, mut host_stream) = host.split();
    let (mut guest_sink, mut guest_stream) = guest.split();
    let bytes_counter = &state.metrics.relay_bytes_forwarded;

    let host_to_guest = async {
        while let Some(Ok(message)) = host_stream.next().await {
            match classify(message, bytes_counter) {
                Forward::Send(frame) => {
                    if guest_sink.send(frame).await.is_err() {
                        break;
                    }
                }
                Forward::Skip => {}
                Forward::Stop => break,
            }
        }
        let _ = guest_sink.send(Message::Close(None)).await;
    };

    let guest_to_host = async {
        while let Some(Ok(message)) = guest_stream.next().await {
            match classify(message, bytes_counter) {
                Forward::Send(frame) => {
                    if host_sink.send(frame).await.is_err() {
                        break;
                    }
                }
                Forward::Skip => {}
                Forward::Stop => break,
            }
        }
        let _ = host_sink.send(Message::Close(None)).await;
    };

    tokio::select! {
        _ = host_to_guest => {}
        _ = guest_to_host => {}
    }
}

/// A peer frame's disposition after counting forwarded bytes.
enum Forward {
    /// Forward this frame to the other side.
    Send(Message),
    /// Drop the frame (e.g. ping/pong) but keep the tunnel open.
    Skip,
    /// The peer closed; tear the tunnel down.
    Stop,
}

/// Classify an inbound relay frame, metering forwarded binary payload bytes.
fn classify(message: Message, bytes_counter: &std::sync::atomic::AtomicU64) -> Forward {
    match message {
        Message::Binary(data) => {
            Metrics::add(bytes_counter, data.len() as u64);
            Forward::Send(Message::Binary(data))
        }
        Message::Text(text) => Forward::Send(Message::Text(text)),
        Message::Close(_) => Forward::Stop,
        _ => Forward::Skip,
    }
}
