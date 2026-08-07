//! In-process per-IP sliding-window rate limiter for the unauthenticated
//! credential endpoints (see [`RateLimiter::throttled_paths`]). State is ephemeral;
//! a poisoned lock is recovered rather than propagated.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use loontail_core::config::RateLimitConfig;
use loontail_core::AppError;

use crate::ip;

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    max_attempts: u32,
    window: Duration,
    trusted_proxy: bool,
    /// Paths that consume a token, matched by equality after `NormalizePathLayer`
    /// has trimmed any trailing slash.
    throttled: Vec<String>,
    hits: Mutex<Buckets>,
    /// Shared bucket for credential requests whose IP can't be resolved, so they
    /// stay throttled (fail-closed) instead of bypassing the limiter.
    unresolved: Mutex<VecDeque<Instant>>,
}

/// The per-key sliding windows plus when the last eviction pass ran, under ONE lock.
///
/// why: `last_sweep` is only ever read/written while holding the map, so giving it its
/// own mutex would only create a second lock a future code path could take in the
/// opposite order — deadlocking the limiter, which sits on every login request.
#[derive(Default)]
struct Buckets {
    map: HashMap<IpAddr, VecDeque<Instant>>,
    last_sweep: Option<Instant>,
}

/// Hard ceiling on tracked keys. At the cap the map evicts its stalest entries to make
/// room, so memory stays bounded AND a newcomer still gets its own budget.
const MAX_TRACKED_IPS: usize = 100_000;

/// How many keys one at-cap eviction pass reclaims. Batching amortises the O(n) recency
/// scan over that many admissions instead of paying it on every request at the cap.
const EVICT_BATCH: usize = MAX_TRACKED_IPS / 10;

impl RateLimiter {
    pub fn from_config(
        config: &RateLimitConfig,
        trusted_proxy: bool,
        yggdrasil_mount_prefix: &str,
    ) -> Self {
        Self::new(
            config.max_attempts,
            config.window,
            trusted_proxy,
            yggdrasil_mount_prefix,
        )
    }

    pub fn new(
        max_attempts: u32,
        window: Duration,
        trusted_proxy: bool,
        yggdrasil_mount_prefix: &str,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_attempts,
                window,
                trusted_proxy,
                throttled: Self::throttled_paths(yggdrasil_mount_prefix),
                hits: Mutex::new(Buckets::default()),
                unresolved: Mutex::new(VecDeque::new()),
            }),
        }
    }

    /// The credential endpoints that consume a token. The Yggdrasil pair is derived
    /// from the router's actual mount prefix — hard-coding `/api/yggdrasil` here
    /// silently unthrottles authenticate/refresh whenever `YGGDRASIL_PUBLIC_URL`
    /// mounts them elsewhere.
    fn throttled_paths(yggdrasil_mount_prefix: &str) -> Vec<String> {
        vec![
            "/admin/auth/login".to_string(),
            "/api/auth/login".to_string(),
            "/api/auth/register".to_string(),
            format!("{yggdrasil_mount_prefix}/authserver/authenticate"),
            format!("{yggdrasil_mount_prefix}/authserver/refresh"),
        ]
    }

    fn is_throttled_path(&self, path: &str) -> bool {
        self.inner.throttled.iter().any(|p| p == path)
    }

    /// Record one attempt for `ip` at `now`; `true` if the count within the trailing
    /// window does not exceed `max_attempts`. Prunes expired timestamps in the same pass
    /// and evicts wholly idle keys at most once per window.
    fn check_at(&self, ip: IpAddr, now: Instant) -> bool {
        let window = self.inner.window;
        let max = self.inner.max_attempts as usize;
        let key = limiter_key(ip);
        let mut buckets = self.inner.hits.lock().unwrap_or_else(|e| e.into_inner());

        let cutoff = now.checked_sub(window).unwrap_or(now);
        buckets.sweep_if_due(now, cutoff, window);

        // why: at the cap the map evicts its stalest keys instead of routing every
        // newcomer into one shared bucket. Sharing would let a client that can mint keys
        // (a proxy that appends X-Forwarded-For) spend the WHOLE deployment's login
        // budget — a global lockout is a far worse failure than the memory it saves.
        if buckets.map.len() >= MAX_TRACKED_IPS && !buckets.map.contains_key(&key) {
            buckets.evict_stalest(EVICT_BATCH);
        }

        let bucket = buckets.map.entry(key).or_default();
        while bucket.front().is_some_and(|&t| t <= cutoff) {
            bucket.pop_front();
        }
        if bucket.len() >= max {
            return false;
        }
        bucket.push_back(now);
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

    #[cfg(test)]
    fn tracked_ips(&self) -> usize {
        self.inner
            .hits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .len()
    }
}

impl Buckets {
    /// Drop every bucket whose NEWEST hit is already outside the window — a key that
    /// never came back. Rate-limited to one pass per window because it walks the whole
    /// map under the lock.
    ///
    /// why: the predicate must be "newest hit is stale", not "bucket is empty". Only
    /// the owning key's own call prunes its deque, so an idle bucket is never empty and
    /// an emptiness test can evict nothing at all.
    fn sweep_if_due(&mut self, now: Instant, cutoff: Instant, window: Duration) {
        let due = match self.last_sweep {
            Some(previous) => now.duration_since(previous) >= window,
            None => true,
        };
        if !due {
            return;
        }
        self.last_sweep = Some(now);
        self.map
            .retain(|_, bucket| bucket.back().is_some_and(|&t| t > cutoff));
    }

    /// Free room at the cap by dropping the `count` least-recently-active keys (a bucket
    /// with no live hit at all is the stalest of all). Losing a bucket only means that
    /// key is untracked again, so it is admitted with a full budget — the same outcome
    /// as the periodic sweep, never a denial.
    fn evict_stalest(&mut self, count: usize) {
        let count = count.min(self.map.len());
        if count == 0 {
            return;
        }
        let mut recency: Vec<(Option<Instant>, IpAddr)> = self
            .map
            .iter()
            .map(|(key, bucket)| (bucket.back().copied(), *key))
            .collect();
        recency.select_nth_unstable(count - 1);
        for (_, key) in &recency[..count] {
            self.map.remove(key);
        }
    }
}

/// The map key for a client address: an IPv4 address as-is, an IPv6 address truncated
/// to its /64 prefix.
///
/// why: a single IPv6 subscriber is routinely delegated a whole /64, so keying on the
/// full address would let ONE client mint 2^64 independent budgets — an unbounded key
/// space and a free bypass of the credential limiter.
fn limiter_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
    }
}

/// `axum::middleware::from_fn_with_state` entry point. Throttles only the
/// credential paths; everything else passes straight through.
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    request: Request,
    next: Next,
) -> Response {
    if !limiter.is_throttled_path(request.uri().path()) {
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
        let limiter = RateLimiter::new(3, Duration::from_secs(60), false, "/api/yggdrasil");
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
        let limiter = RateLimiter::new(2, Duration::from_secs(60), false, "/api/yggdrasil");
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
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false, "/api/yggdrasil");
        let now = Instant::now();
        assert!(limiter.check_at(ip(3), now));
        assert!(!limiter.check_at(ip(3), now));
        assert!(
            limiter.check_at(ip(4), now),
            "a different IP has its own budget"
        );
    }

    /// BEQ-19: an IP that never comes back must actually be evicted. The old
    /// `retain(|_, b| !b.is_empty())` predicate was unsatisfiable — a bucket is only
    /// ever pruned by its own owner's next call — so the map grew forever.
    #[test]
    fn idle_ips_are_evicted_by_the_periodic_sweep() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(5, window, false, "/api/yggdrasil");
        let t0 = Instant::now();
        for last in 10..13 {
            assert!(limiter.check_at(ip(last), t0));
        }
        assert_eq!(limiter.tracked_ips(), 3, "three IPs tracked at t0");

        // Two windows later the three t0 buckets are wholly stale; checking a fourth IP
        // triggers the due sweep, which must drop them and leave only the newcomer.
        assert!(limiter.check_at(ip(13), t0 + window * 2));
        assert_eq!(
            limiter.tracked_ips(),
            1,
            "stale buckets evicted; only the IP just checked remains"
        );
    }

    /// The sweep must not cost an IP its budget: an evicted IP is simply untracked, so
    /// its next request starts from a full window.
    #[test]
    fn swept_ip_gets_a_fresh_budget() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(1, window, false, "/api/yggdrasil");
        let t0 = Instant::now();
        let addr = ip(20);
        assert!(limiter.check_at(addr, t0));
        assert!(!limiter.check_at(addr, t0), "budget of 1 spent");

        let later = t0 + window * 2;
        assert!(
            limiter.check_at(ip(21), later),
            "unrelated IP triggers sweep"
        );
        assert!(
            limiter.check_at(addr, later),
            "swept IP is admitted again with a full budget"
        );
    }

    /// The sweep runs at most once per window, so a burst inside one window does not
    /// walk the whole map per request — and buckets from that same window survive it.
    #[test]
    fn sweep_is_rate_limited_to_one_pass_per_window() {
        let window = Duration::from_secs(60);
        let limiter = RateLimiter::new(5, window, false, "/api/yggdrasil");
        let t0 = Instant::now();
        assert!(limiter.check_at(ip(30), t0));
        // Half a window later: not due, so ip(30)'s live bucket is untouched.
        assert!(limiter.check_at(ip(31), t0 + window / 2));
        assert_eq!(limiter.tracked_ips(), 2, "both live buckets kept");
    }

    /// At the cap the limiter must EVICT, not share. Routing every newcomer into one
    /// shared bucket turned a bounded-memory guard into a deployment-wide login lockout:
    /// the first fresh IP spent the shared budget and every following one got a 429.
    #[test]
    fn at_cap_fresh_ips_keep_independent_budgets() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false, "/api/yggdrasil");
        let now = Instant::now();
        // Fill the map to exactly the cap, all inside one window so the periodic sweep
        // cannot reclaim any of it.
        for n in 0..MAX_TRACKED_IPS as u32 {
            assert!(limiter.check_at(IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + n)), now));
        }
        assert_eq!(limiter.tracked_ips(), MAX_TRACKED_IPS, "map is at the cap");

        let fresh_a = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
        let fresh_b = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));
        assert!(
            limiter.check_at(fresh_a, now),
            "a fresh IP at the cap is still admitted"
        );
        assert!(
            limiter.check_at(fresh_b, now),
            "and with its OWN budget — one newcomer must not spend another's"
        );
        assert!(
            !limiter.check_at(fresh_b, now),
            "its own budget of 1 is still enforced"
        );
        assert_eq!(
            limiter.tracked_ips(),
            MAX_TRACKED_IPS - EVICT_BATCH + 2,
            "one batch was evicted to make room; memory stays bounded"
        );
    }

    /// IPv6 is keyed by /64: a subscriber holding a whole /64 must not be able to mint
    /// 2^64 independent login budgets (nor 2^64 map entries).
    #[test]
    fn ipv6_addresses_in_one_64_share_a_budget() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false, "/api/yggdrasil");
        let now = Instant::now();
        let first: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let sibling: IpAddr = "2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap();
        let other_prefix: IpAddr = "2001:db8:1:3::1".parse().unwrap();

        assert!(limiter.check_at(first, now));
        assert!(
            !limiter.check_at(sibling, now),
            "same /64 shares one budget"
        );
        assert!(
            limiter.check_at(other_prefix, now),
            "a different /64 keeps its own budget"
        );
        assert_eq!(
            limiter.tracked_ips(),
            2,
            "two /64 keys, not three addresses"
        );
    }

    #[test]
    fn unresolved_ip_fails_closed_into_shared_bucket() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false, "/api/yggdrasil");
        let now = Instant::now();
        assert!(limiter.check_unresolved_at(now), "first unresolved allowed");
        assert!(
            !limiter.check_unresolved_at(now),
            "second unresolved blocked — fail-closed, not passthrough"
        );
    }

    #[test]
    fn throttled_path_matching() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false, "/api/yggdrasil");
        assert!(limiter.is_throttled_path("/api/auth/login"));
        assert!(limiter.is_throttled_path("/admin/auth/login"));
        assert!(limiter.is_throttled_path("/api/auth/register"));
        assert!(limiter.is_throttled_path("/api/yggdrasil/authserver/authenticate"));
        assert!(limiter.is_throttled_path("/api/yggdrasil/authserver/refresh"));
        assert!(!limiter.is_throttled_path("/api/auth/me"));
        assert!(!limiter.is_throttled_path("/health"));
    }

    /// BE-03: the Yggdrasil credential paths follow the configured mount, so a
    /// non-default `YGGDRASIL_PUBLIC_URL` cannot silently unthrottle them.
    #[test]
    fn throttled_paths_follow_the_yggdrasil_mount() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60), false, "/api/mojang");
        assert!(limiter.is_throttled_path("/api/mojang/authserver/authenticate"));
        assert!(limiter.is_throttled_path("/api/mojang/authserver/refresh"));
        assert!(
            !limiter.is_throttled_path("/api/yggdrasil/authserver/authenticate"),
            "the default path is not mounted under this config"
        );
        assert!(
            limiter.is_throttled_path("/api/auth/login"),
            "non-yggdrasil credential paths stay literal"
        );
    }

    /// BE-03 end-to-end: with a non-default Yggdrasil mount, `authenticate` still runs
    /// out of budget and answers 429 through the real middleware.
    #[tokio::test]
    async fn authenticate_is_throttled_through_the_middleware() {
        use axum::body::Body;
        use axum::http::{Request as HttpRequest, StatusCode};
        use axum::routing::post;
        use tower::util::ServiceExt;

        let limiter = RateLimiter::new(2, Duration::from_secs(60), false, "/api/mojang");
        let app = axum::Router::new()
            .route(
                "/api/mojang/authserver/authenticate",
                post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn_with_state(
                limiter,
                rate_limit_middleware,
            ));

        let mut statuses = Vec::new();
        for _ in 0..3 {
            let resp = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/api/mojang/authserver/authenticate")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            statuses.push(resp.status());
        }
        assert_eq!(statuses[0], StatusCode::OK);
        assert_eq!(statuses[1], StatusCode::OK);
        assert_eq!(
            statuses[2],
            StatusCode::TOO_MANY_REQUESTS,
            "max_attempts + 1 must be rejected at the configured mount"
        );
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
