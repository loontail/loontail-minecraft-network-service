use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::auth;
use crate::error::{AppError, AppResult};
use crate::join_requests::JoinTicketDto;
use crate::models::UserDto;
use crate::state::AppState;

/// Events pushed from the service to a connected client over its signaling
/// WebSocket. Serialized as `{ "type": "...", ... }`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerEvent {
    FriendRequest {
        request_id: Uuid,
        from_user: UserDto,
    },
    FriendRequestAccepted {
        request_id: Uuid,
        by_user: UserDto,
    },
    JoinRequest {
        request_id: Uuid,
        world_session_id: Uuid,
        from_user: UserDto,
    },
    /// Delivered to the guest: contains the join ticket needed to open relay.
    JoinRequestAccepted {
        request_id: Uuid,
        ticket: JoinTicketDto,
    },
    JoinRequestDeclined {
        request_id: Uuid,
    },
    /// Delivered to the host: a guest is about to connect; open the relay
    /// tunnel for `relay_session_id`.
    GuestArriving {
        relay_session_id: Uuid,
        world_session_id: Uuid,
        guest_user: UserDto,
    },
    /// Delivered to the invitee: someone invited them into a world.
    WorldInvite {
        invite_id: Uuid,
        world_session_id: Uuid,
        inviter: UserDto,
        host: UserDto,
    },
    /// Delivered to the host: a friend-of-friend invite needs their approval.
    InviteApprovalRequest {
        invite_id: Uuid,
        world_session_id: Uuid,
        inviter: UserDto,
        invitee: UserDto,
    },
    /// Delivered to the requester: their friend request was declined.
    FriendRequestDeclined {
        request_id: Uuid,
    },
    /// Delivered to a user that was removed from someone's friend list.
    FriendRemoved {
        user_id: Uuid,
    },
    /// Delivered to a user's friends when their presence changes; the recipient
    /// should reload their friends list to pick up the new status.
    PresenceUpdate {
        user_id: Uuid,
    },
    /// Delivered to a world's active guests when the host changes the invite
    /// policy, so a guest's own "invite a friend" affordance turns on/off live.
    WorldPolicyChanged {
        world_session_id: Uuid,
        invite_policy: String,
    },
}

type Connection = (Uuid, mpsc::UnboundedSender<ServerEvent>);

/// Fan-out hub keyed by user id. A user may have multiple live connections.
pub struct SignalingHub {
    connections: Mutex<HashMap<Uuid, Vec<Connection>>>,
}

impl SignalingHub {
    pub fn new() -> Self {
        SignalingHub {
            connections: Mutex::new(HashMap::new()),
        }
    }

    fn add(&self, user_id: Uuid) -> (Uuid, mpsc::UnboundedReceiver<ServerEvent>) {
        let conn_id = Uuid::new_v4();
        let (tx, rx) = mpsc::unbounded_channel();
        let mut map = self.connections.lock().unwrap();
        map.entry(user_id).or_default().push((conn_id, tx));
        (conn_id, rx)
    }

    fn remove(&self, user_id: Uuid, conn_id: Uuid) {
        let mut map = self.connections.lock().unwrap();
        if let Some(conns) = map.get_mut(&user_id) {
            conns.retain(|(id, _)| *id != conn_id);
            if conns.is_empty() {
                map.remove(&user_id);
            }
        }
    }

    /// Deliver an event to all of a user's live connections. Dead senders are
    /// pruned. No-op (and harmless) when the user is offline.
    pub fn send_to(&self, user_id: Uuid, event: ServerEvent) {
        let mut map = self.connections.lock().unwrap();
        if let Some(conns) = map.get_mut(&user_id) {
            conns.retain(|(_, tx)| tx.send(event.clone()).is_ok());
            if conns.is_empty() {
                map.remove(&user_id);
            }
        }
    }

    pub fn connected_users(&self) -> usize {
        self.connections.lock().unwrap().len()
    }

    /// Whether the user currently has any live signaling connection.
    pub fn is_online(&self, user_id: Uuid) -> bool {
        self.connections.lock().unwrap().contains_key(&user_id)
    }
}

impl Default for SignalingHub {
    fn default() -> Self {
        SignalingHub::new()
    }
}

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
    crate::metrics::Metrics::incr(&state.metrics.signaling_connections);

    Ok(ws.on_upgrade(move |socket| handle_signaling(socket, state, user_id)))
}

async fn handle_signaling(socket: WebSocket, state: AppState, user_id: Uuid) {
    let (mut sink, mut stream) = socket.split();
    let (conn_id, mut rx) = state.signaling.add(user_id);

    // Coming online: mark live and tell this user's friends to refresh.
    let _ = crate::presence::mark_online(&state, user_id).await;
    let _ = crate::presence::broadcast_presence(&state, user_id).await;

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

    state.signaling.remove(user_id, conn_id);

    // Last connection closed: go offline and tell friends to refresh.
    if !state.signaling.is_online(user_id) {
        let _ = crate::presence::mark_offline(&state, user_id).await;
        let _ = crate::presence::broadcast_presence(&state, user_id).await;
    }
}
