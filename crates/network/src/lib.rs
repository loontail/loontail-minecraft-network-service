//! Network domain: friends, presence, world sessions, the join flow, relay and signaling.

pub mod cleanup;
pub mod friends;
pub mod invites;
pub mod join;
pub mod presence;
pub(crate) mod queries;
pub mod relay;
pub mod signaling;
pub mod users;
pub mod worlds;

use axum::routing::{delete, get, patch, post};
use axum::Router;

use loontail_core::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/users/bootstrap", post(users::bootstrap))
        .route("/me", get(users::me))
        .route("/users/search", get(users::search))
        .route("/friends", get(friends::list_friends))
        .route("/friends/requests", post(friends::create_request))
        .route("/friends/requests/incoming", get(friends::incoming))
        .route("/friends/requests/outgoing", get(friends::outgoing))
        .route("/friends/requests/{id}/accept", post(friends::accept))
        .route("/friends/requests/{id}/decline", post(friends::decline))
        .route("/friends/{user_id}", delete(friends::remove_friend))
        .route("/presence/heartbeat", post(presence::heartbeat))
        .route("/presence/status", post(presence::set_status))
        .route("/presence/friends", get(presence::friends_presence))
        .route("/world-sessions", post(worlds::create))
        .route(
            "/world-sessions/{id}",
            patch(worlds::update).delete(worlds::close),
        )
        .route(
            "/world-sessions/{id}/join-ticket",
            post(join::create_join_ticket),
        )
        .route(
            "/world-sessions/{id}/join-requests",
            post(join::create_join_request),
        )
        .route("/join-requests/incoming", get(join::incoming))
        .route("/join-requests/{id}/accept", post(join::accept))
        .route("/join-requests/{id}/decline", post(join::decline))
        .route("/world-sessions/{id}/invites", post(invites::create))
        .route("/invites/incoming", get(invites::incoming))
        .route("/invites/outgoing", get(invites::outgoing))
        .route("/invites/pending-approval", get(invites::pending_approval))
        .route("/invites/{id}/accept", post(invites::accept))
        .route("/invites/{id}/decline", post(invites::decline))
        .route("/invites/{id}/approve", post(invites::approve))
        .route("/invites/{id}", delete(invites::revoke))
        .route("/signaling", get(signaling::signaling_ws))
        .route("/relay/{relay_session_id}", get(relay::relay_ws))
}
