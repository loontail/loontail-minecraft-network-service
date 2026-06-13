//! In-process per-IP sliding-window rate limiter for unauthenticated credential
//! endpoints (login/register/Yggdrasil authenticate+refresh).
//!
//! No external crate: a `Mutex<HashMap<IpAddr, VecDeque<Instant>>>` records each
//! IP's recent attempt timestamps and rejects once more than `max_attempts` fall
//! inside the trailing `window`. State is purely ephemeral; a poisoned lock is
//! recovered rather than propagated. The middleware self-filters to the exact
//! credential paths so every other route is untouched.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use loontail_core::config::RateLimitConfig;
use loontail_core::AppError;

/// Paths that consume a token from the limiter. Matched after `NormalizePathLayer`
/// has trimmed any trailing slash, so the bare forms below suffice.
const THROTTLED_PATHS: &[&str] = &[
    "/admin/auth/login",
    "/api/auth/login",
    "/api/auth/register",
    "/api/yggdrasil/authserver/authenticate",
    "/api/yggdrasil/authserver/refresh",
];

/// Shared per-IP sliding-window limiter. Cheap to clone (an `Arc`).
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    max_attempts: u32,
    window: Duration,
    hits: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn from_config(config: &RateLimitConfig) -> Self {
        Self::new(config.max_attempts, config.window)
    }

    pub fn new(max_attempts: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_attempts,
                window,
                hits: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Record one attempt for `ip` at `now`; return `true` if it is allowed (i.e.
    /// the count within the trailing window does not exceed `max_attempts`).
    /// Expired timestamps are pruned in the same pass and empty buckets dropped.
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
}

/// Resolve the client IP, preferring the transport peer from `ConnectInfo`. Falls
/// back to the first hop of `X-Forwarded-For` only when there is no peer address
/// (e.g. behind a TRUSTED reverse proxy that strips the socket). The header is
/// client-controllable, so this path must only be reachable behind a proxy that
/// overwrites `X-Forwarded-For`.
fn resolve_ip(connect_info: Option<&IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(ip) = connect_info {
        return Some(*ip);
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.split(',').next())
        .and_then(|first| first.trim().parse::<IpAddr>().ok())
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
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    if let Some(ip) = resolve_ip(peer.as_ref(), request.headers()) {
        if !limiter.check(ip) {
            tracing::warn!(%ip, path = request.uri().path(), "rate limit exceeded");
            return AppError::TooManyRequests.into_response();
        }
    }
    // why: when no IP can be resolved we fail OPEN rather than block every caller;
    // in production `ConnectInfo` always yields the peer, so this is dev-only.
    next.run(request).await
}

fn is_throttled_path(path: &str) -> bool {
    THROTTLED_PATHS.contains(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
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
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
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
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check_at(ip(3), now));
        assert!(!limiter.check_at(ip(3), now));
        assert!(
            limiter.check_at(ip(4), now),
            "a different IP has its own budget"
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

    #[test]
    fn connect_info_ip_is_preferred() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "9.9.9.9".parse().unwrap());
        let peer = ip(5);
        assert_eq!(resolve_ip(Some(&peer), &headers), Some(peer));
    }

    #[test]
    fn forwarded_for_first_hop_fallback() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        assert_eq!(
            resolve_ip(None, &headers),
            Some(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)))
        );
    }

    #[test]
    fn no_ip_when_neither_source_present() {
        let headers = HeaderMap::new();
        assert_eq!(resolve_ip(None, &headers), None);
    }
}
