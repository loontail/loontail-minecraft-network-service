use axum::http::HeaderValue;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;
use crate::{
    friends, health, invites, join_requests, metrics, presence, relay, signaling, users, worlds,
};

/// Build the full application router with CORS and HTTP tracing.
pub fn build_router(state: AppState) -> Router {
    let cors = build_cors(&state.config.cors_allowed_origins);

    Router::new()
        // Infrastructure
        .route("/health", get(health::health))
        .route("/metrics", get(metrics::metrics_handler))
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
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let base = CorsLayer::new().allow_methods(Any).allow_headers(Any);
    if allowed_origins.iter().any(|origin| origin == "*") {
        base.allow_origin(Any)
    } else {
        let origins: Vec<HeaderValue> = allowed_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        base.allow_origin(origins)
    }
}
