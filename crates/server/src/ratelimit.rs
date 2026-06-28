//! In-process per-IP sliding-window rate limiter for the unauthenticated
//! credential endpoints (see [`THROTTLED_PATHS`]). State is ephemeral; a poisoned
//! lock is recovered rather than propagated.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use loontail_core::config::RateLimitConfig;
use loontail_core::AppError;

use crate::ip;

/// Paths that consume a token from the limiter. Matched after `NormalizePathLayer`
/// has trimmed any trailing slash, so the bare forms below suffice.
const THROTTLED_PATHS: &[&str] = &[
    "/admin/auth/login",
    "/api/auth/login",
    "/api/auth/register",
    "/api/yggdrasil/authserver/authenticate",
    "/api/yggdrasil/authserver/refresh",
];

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    max_attempts: u32,
    window: Duration,
    trusted_proxy: bool,
    hits: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    /// Shared bucket for credential requests whose IP can't be resolved, so they
    /// stay throttled (fail-closed) instead of bypassing the limiter.
    unresolved: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn from_config(config: &RateLimitConfig, trusted_proxy: bool) -> Self {
        Self::new(config.max_attempts, config.window, trusted_proxy)
    }

    pub fn new(max_attempts: u32, window: Duration, trusted_proxy: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_attempts,
                window,
                trusted_proxy,
                hits: Mutex::new(HashMap::new()),
                unresolved: Mutex::new(VecDeque::new()),
            }),
        }
    }

    /// Record one attempt for `ip` at `now`; `true` if the count within the trailing
    /// window does not exceed `max_attempts`. Prunes expired timestamps in the same pass.
    fn check_at(&self, ip: IpAddr, now: Instant) -> bool {
        let window = self.inner.window;
        let max = self.inner.max_attempts as usize;
        let mut map = self.inner.hits.lock().unwrap_or_else(|e| e.into_inner());

        let cutoff = now.checked_sub(window).unwrap_or(now);
        let bucket = map.entry(ip).or_default();
        while bucket.front().is_some_and(|&t| t <= cutoff) {
            bucket.pop_front();
        }
        if bucket.len() >= max {
            return false;
        }
        bucket.push_back(now);

        // Opportunistic sweep so idle IPs don't accumulate empty buckets forever.
        map.retain(|_, b| !b.is_empty());
        true
    }

    fn check(&self, ip: IpAddr) -> bool {
        self.check_at(ip, Instant::now())
    }

    /// Fail-closed bucket for credential requests with an unresolvable IP: they
    /// all share one conservative budget so a missing IP can never bypass the
    /// limiter on the very endpoints worth protecting (SEC-3).
    fn check_unresolved_at(&self, now: Instant) -> bool {
        let window = self.inner.window;
        let max = self.inner.max_attempts as usize;
        let mut bucket = self
            .inner
            .unresolved
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let cutoff = now.checked_sub(window).unwrap_or(now);
        while bucket.front().is_some_and(|&t| t <= cutoff) {
            bucket.pop_front();
        }
        if bucket.len() >= max {
            return false;
        }
        bucket.push_back(now);
        true
    }

    fn check_unresolved(&self) -> bool {
        self.check_unresolved_at(Instant::now())
    }
}

/// `axum::middleware::from_fn_with_state` entry point. Throttles only the
/// credential paths; everything else passes straight through.
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    if !is_throttled_path(request.uri().path()) {
        return next.run(request).await;
    }

    // `into_make_service_with_connect_info::<SocketAddr>()` stashes the peer in
    // request extensions; read it there to avoid an extractor-tuple bound.
    let peer = ip::peer_from_extensions(request.extensions().get::<ConnectInfo<SocketAddr>>());
    let resolved = ip::client_ip(request.headers(), peer, limiter.inner.trusted_proxy);

    let allowed = match resolved {
        Some(ip) => {
            let ok = limiter.check(ip);
            if !ok {
                tracing::warn!(%ip, path = request.uri().path(), "rate limit exceeded");
            }
            ok
        }
        // why: a credential path with no resolvable IP fails CLOSED into a shared
        // conservative bucket rather than passing through unthrottled — otherwise a
        // missing/forged source would defeat the limiter on exactly the endpoints
        // that matter.
        None => {
            let ok = limiter.check_unresolved();
            if !ok {
                tracing::warn!(
                    path = request.uri().path(),
                    "rate limit exceeded (unresolved client IP)"
                );
            }
            ok
        }
    };

    if !allowed {
        return AppError::TooManyRequests.into_response();
    }
    next.run(request).await
}

fn is_throttled_path(path: &str) -> bool {
    THROTTLED_PATHS.contains(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ip::client_ip;
    use axum::http::HeaderMap;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60), false);
        let now = Instant::now();
        let addr = ip(1);
        assert!(limiter.check_at(addr, now));
        assert!(limiter.check_at(addr, now));
        assert!(limiter.check_at(addr, now));
        assert!(
            !limiter.check_at(addr, now),
            "fourth attempt within the window is rejected"
        );
    }

    #[test]
    fn window_slides_and_frees_slots() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60), false);
        let t0 = Instant::now();
        let addr = ip(2);
        assert!(limiter.check_at(addr, t0));
        assert!(limiter.check_at(addr, t0));
        assert!(!limiter.check_at(addr, t0), "limit reached");
        // Past the window the earliest hits expire, freeing capacity.
        let later = t0 + Duration::from_secs(61);
        assert!(limiter.check_at(addr, later), "slot freed after window");
    }

    #[test]
    fn ips_are_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false);
        let now = Instant::now();
        assert!(limiter.check_at(ip(3), now));
        assert!(!limiter.check_at(ip(3), now));
        assert!(
            limiter.check_at(ip(4), now),
            "a different IP has its own budget"
        );
    }

    #[test]
    fn unresolved_ip_fails_closed_into_shared_bucket() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false);
        let now = Instant::now();
        assert!(limiter.check_unresolved_at(now), "first unresolved allowed");
        assert!(
            !limiter.check_unresolved_at(now),
            "second unresolved blocked — fail-closed, not passthrough"
        );
    }

    #[test]
    fn throttled_path_matching() {
        assert!(is_throttled_path("/api/auth/login"));
        assert!(is_throttled_path("/admin/auth/login"));
        assert!(is_throttled_path("/api/yggdrasil/authserver/authenticate"));
        assert!(is_throttled_path("/api/yggdrasil/authserver/refresh"));
        assert!(!is_throttled_path("/api/auth/me"));
        assert!(!is_throttled_path("/health"));
    }

    /// With a peer present AND `trusted_proxy=true`, the limiter key is the XFF hop,
    /// not the peer; with `trusted_proxy=false` it is the peer and XFF is ignored.
    /// Asserted at the key-derivation level (`client_ip`) the limiter consumes.
    #[test]
    fn limiter_key_follows_trusted_proxy_flag() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
        let peer = ip(5);

        // trusted_proxy=true: key is the XFF hop, NOT the peer.
        let key_trusted = client_ip(&headers, Some(peer), true);
        assert_eq!(
            key_trusted,
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))),
            "trusted proxy → bucket by the XFF hop"
        );
        assert_ne!(
            key_trusted,
            Some(peer),
            "trusted proxy must not key on the peer"
        );

        // trusted_proxy=false: key is the peer, XFF ignored entirely.
        assert_eq!(
            client_ip(&headers, Some(peer), false),
            Some(peer),
            "untrusted → bucket by the peer, XFF ignored"
        );

        // The two keys differ, so a per-IP limiter buckets them separately — the
        // whole point of SEC-1.
        assert_ne!(key_trusted, client_ip(&headers, Some(peer), false));
    }
}
