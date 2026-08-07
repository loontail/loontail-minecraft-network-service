use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use loontail_core::auth;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;
use loontail_core::Metrics;

use crate::presence;

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    /// Optional `?token=` fallback; the Bearer header is preferred.
    pub token: Option<String>,
}

/// `GET /signaling` — upgrade to the per-user signaling WebSocket. Auth comes
/// from the `Authorization: Bearer` header (like REST), with a `?token=` fallback.
pub async fn signaling_ws(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    // Authenticate before upgrading so failures are plain HTTP responses.
    let token = auth::bearer_token_from_headers(&headers)
        .or(query.token)
        .ok_or(AppError::Unauthorized)?;
    let user = auth::user_from_token(&state.pool, &token).await?;
    let user_id = user.id;
    Metrics::incr(&state.metrics.signaling_connections);

    Ok(ws.on_upgrade(move |socket| handle_signaling(socket, state, user_id)))
}

async fn handle_signaling(socket: WebSocket, state: AppState, user_id: Uuid) {
    let (mut sink, mut stream) = socket.split();
    let (conn_id, mut rx) = state.realtime.signaling.add(user_id);

    // Coming online: mark live and tell this user's friends to refresh.
    if let Err(e) = presence::mark_online(&state, user_id).await {
        tracing::warn!(error = %e, %user_id, "failed to mark user online on signaling connect");
    }
    if let Err(e) = presence::broadcast_presence(&state, user_id).await {
        tracing::warn!(error = %e, %user_id, "failed to broadcast presence on signaling connect");
    }

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(event) => {
                        let payload = match serde_json::to_string(&event) {
                            Ok(payload) => payload,
                            Err(_) => continue,
                        };
                        if sink.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            incoming = stream.next() => {
                match incoming {
                    // Clients may send pings/heartbeats; we just keep the socket alive.
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }

    state.realtime.signaling.remove(user_id, conn_id);

    // Last connection closed: go offline and tell friends to refresh.
    if !state.realtime.signaling.is_online(user_id) {
        if let Err(e) = presence::mark_offline(&state, user_id).await {
            tracing::warn!(error = %e, %user_id, "failed to mark user offline on signaling disconnect");
        }
        if let Err(e) = presence::broadcast_presence(&state, user_id).await {
            tracing::warn!(error = %e, %user_id, "failed to broadcast presence on signaling disconnect");
        }
    }
}
