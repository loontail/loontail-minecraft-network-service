//! Network domain: friends, presence, world sessions, the join flow, relay and
//! signaling. Depends only on `loontail-core` and exposes `routes()` returning a
//! `Router<AppState>` the server crate merges in.

pub mod friends;
pub mod invites;
pub mod join_requests;
pub mod presence;
pub mod relay;
pub mod signaling;
pub mod users;
pub mod worlds;

use axum::routing::{delete, get, patch, post};
use axum::Router;

use loontail_core::AppState;

/// Build the network domain router. Infrastructure routes (`/health`,
/// `/metrics`) and middleware (CORS, tracing) are applied by the server crate.
pub fn routes() -> Router<AppState> {
    Router::new()
        // Bootstrap + current user
        .route("/users/bootstrap", post(users::bootstrap))
        .route("/me", get(users::me))
        .route("/users/search", get(users::search))
        // Friends
        .route("/friends", get(friends::list_friends))
        .route("/friends/requests", post(friends::create_request))
        .route("/friends/requests/incoming", get(friends::incoming))
        .route("/friends/requests/outgoing", get(friends::outgoing))
        .route("/friends/requests/{id}/accept", post(friends::accept))
        .route("/friends/requests/{id}/decline", post(friends::decline))
        .route("/friends/{user_id}", delete(friends::remove_friend))
        // Presence
        .route("/presence/heartbeat", post(presence::heartbeat))
        .route("/presence/status", post(presence::set_status))
        .route("/presence/friends", get(presence::friends_presence))
        // World sessions
        .route("/world-sessions", post(worlds::create))
        .route(
            "/world-sessions/{id}",
            patch(worlds::update).delete(worlds::close),
        )
        // Join
        .route(
            "/world-sessions/{id}/join-ticket",
            post(join_requests::create_join_ticket),
        )
        .route(
            "/world-sessions/{id}/join-requests",
            post(join_requests::create_join_request),
        )
        .route("/join-requests/incoming", get(join_requests::incoming))
        .route("/join-requests/{id}/accept", post(join_requests::accept))
        .route("/join-requests/{id}/decline", post(join_requests::decline))
        // Invites
        .route("/world-sessions/{id}/invites", post(invites::create))
        .route("/invites/incoming", get(invites::incoming))
        .route("/invites/outgoing", get(invites::outgoing))
        .route("/invites/pending-approval", get(invites::pending_approval))
        .route("/invites/{id}/accept", post(invites::accept))
        .route("/invites/{id}/decline", post(invites::decline))
        .route("/invites/{id}/approve", post(invites::approve))
        .route("/invites/{id}", delete(invites::revoke))
        // Signaling + relay (WebSocket)
        .route("/signaling", get(signaling::signaling_ws))
        .route("/relay/{relay_session_id}", get(relay::relay_ws))
}
