//! In-memory realtime state shared across the network domain: the per-user
//! signaling fan-out registry and the relay rendezvous map.
//!
//! This module owns only the generic data structures and their
//! register/send/take operations — no axum WS handlers and no network business
//! logic — so it stays free of dependency cycles. The domain crate's WS
//! handlers drive these structures.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use axum::extract::ws::WebSocket;
use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::models::{JoinTicketDto, UserDto};

/// Events pushed from the service to a connected client over its signaling
/// WebSocket. Serialized as `{ "type": "...", ... }`.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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

    /// Register a new connection for a user; returns its id and the receiver the
    /// caller pumps to the socket.
    pub fn add(&self, user_id: Uuid) -> (Uuid, mpsc::UnboundedReceiver<ServerEvent>) {
        let conn_id = Uuid::new_v4();
        let (tx, rx) = mpsc::unbounded_channel();
        let mut map = self.connections.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(user_id).or_default().push((conn_id, tx));
        (conn_id, rx)
    }

    /// Drop a previously registered connection.
    pub fn remove(&self, user_id: Uuid, conn_id: Uuid) {
        let mut map = self.connections.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut map = self.connections.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(conns) = map.get_mut(&user_id) {
            conns.retain(|(_, tx)| tx.send(event.clone()).is_ok());
            if conns.is_empty() {
                map.remove(&user_id);
            }
        }
    }

    pub fn connected_users(&self) -> usize {
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Whether the user currently has any live signaling connection.
    pub fn is_online(&self, user_id: Uuid) -> bool {
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&user_id)
    }
}

impl Default for SignalingHub {
    fn default() -> Self {
        SignalingHub::new()
    }
}

/// The first party of a relay pair, parked until its peer arrives. The handler
/// pipes `socket` to the second party's socket and signals `done` when finished.
pub struct PendingPeer {
    pub socket: WebSocket,
    pub done: tokio::sync::oneshot::Sender<()>,
}

/// Rendezvous hub: holds the first party of each relay pair until the second
/// arrives.
pub struct RelayHub {
    waiting: Mutex<HashMap<Uuid, PendingPeer>>,
    active: AtomicUsize,
}

impl RelayHub {
    pub fn new() -> Self {
        RelayHub {
            waiting: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
        }
    }

    /// Park the first party's socket at the rendezvous keyed by relay session.
    pub fn park(&self, relay_session_id: Uuid, peer: PendingPeer) {
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(relay_session_id, peer);
    }

    /// Take whichever party (if any) is already waiting at the rendezvous.
    pub fn take(&self, relay_session_id: Uuid) -> Option<PendingPeer> {
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&relay_session_id)
    }

    pub fn active_pairings(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    pub fn incr_active(&self) {
        self.active.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decr_active(&self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for RelayHub {
    fn default() -> Self {
        RelayHub::new()
    }
}

/// Aggregate of the in-memory realtime structures held by `AppState`.
pub struct Realtime {
    pub signaling: SignalingHub,
    pub relay: RelayHub,
}

impl Realtime {
    pub fn new() -> Self {
        Realtime {
            signaling: SignalingHub::new(),
            relay: RelayHub::new(),
        }
    }
}

impl Default for Realtime {
    fn default() -> Self {
        Realtime::new()
    }
}
