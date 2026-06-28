//! Single source of truth for resolving a request's client IP.
//!
//! IP precedence is security-sensitive (the rate limiter and request log both key
//! on it), so it lives in exactly one place rather than being re-derived per
//! middleware (SRV-DUP-2).
//!
//! Precedence:
//! - `trusted_proxy == false` (default): use the transport peer from
//!   `ConnectInfo`. Forwarding headers are NEVER trusted — they are
//!   client-controllable, so honouring them without a trusted proxy lets an
//!   attacker rotate the header for a fresh rate-limit bucket per request
//!   (SEC-3).
//! - `trusted_proxy == true`: the immediate peer is a proxy we control that
//!   overwrites `X-Forwarded-For`. Take the right-most hop of `X-Forwarded-For`
//!   (the address the trusted proxy actually saw — earlier hops are
//!   client-supplied and forgeable), falling back to `X-Real-IP`, then to the
//!   peer. (SEC-1)

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;

/// Resolve the client IP for `headers`/`peer_addr` under the configured proxy
/// trust. Returns `None` only when no source yields a parseable IP (e.g. no peer
/// and, when trusted, no usable forwarding header) — callers on credential paths
/// must treat `None` conservatively (fail-closed) rather than as "unthrottled".
pub fn client_ip(
    headers: &HeaderMap,
    peer_addr: Option<IpAddr>,
    trusted_proxy: bool,
) -> Option<IpAddr> {
    if !trusted_proxy {
        // Default: only the transport peer is trustworthy. XFF is ignored.
        return peer_addr;
    }

    // Behind a trusted proxy: the right-most XFF hop is the address the proxy
    // itself observed. Anything to the left of it is appended by upstream hops
    // (ultimately the client) and is forgeable, so we never read the left-most.
    if let Some(ip) = forwarded_for_rightmost(headers) {
        return Some(ip);
    }
    if let Some(ip) = real_ip(headers) {
        return Some(ip);
    }
    peer_addr
}

/// Pull the peer `IpAddr` out of the `ConnectInfo<SocketAddr>` axum stashes in
/// request extensions via `into_make_service_with_connect_info`.
pub fn peer_from_extensions(connect_info: Option<&ConnectInfo<SocketAddr>>) -> Option<IpAddr> {
    connect_info.map(|ConnectInfo(addr)| addr.ip())
}

/// The right-most parseable hop of `X-Forwarded-For`. Behind a trusted proxy this
/// is the address that proxy saw the request arrive from.
fn forwarded_for_rightmost(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())?
        .split(',')
        .rev()
        .find_map(|hop| hop.trim().parse::<IpAddr>().ok())
}

fn real_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.trim().parse::<IpAddr>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, last))
    }

    fn xff(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", value.parse().unwrap());
        h
    }

    #[test]
    fn untrusted_uses_peer_and_ignores_xff() {
        let headers = xff("1.2.3.4, 9.9.9.9");
        let peer = ip(5);
        // XFF is present but must be ignored: the peer wins.
        assert_eq!(client_ip(&headers, Some(peer), false), Some(peer));
    }

    #[test]
    fn untrusted_never_falls_back_to_xff() {
        let headers = xff("1.2.3.4");
        // No peer + untrusted → no IP at all (XFF is client-controllable).
        assert_eq!(client_ip(&headers, None, false), None);
    }

    #[test]
    fn trusted_takes_rightmost_xff_hop_over_peer() {
        let headers = xff("1.1.1.1, 2.2.2.2, 3.3.3.3");
        let peer = ip(5);
        // Right-most hop = what the trusted proxy actually saw.
        assert_eq!(
            client_ip(&headers, Some(peer), true),
            Some(IpAddr::V4(Ipv4Addr::new(3, 3, 3, 3)))
        );
    }

    #[test]
    fn trusted_falls_back_to_real_ip_then_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "4.4.4.4".parse().unwrap());
        assert_eq!(
            client_ip(&headers, Some(ip(5)), true),
            Some(IpAddr::V4(Ipv4Addr::new(4, 4, 4, 4)))
        );

        // No forwarding headers at all → peer.
        let empty = HeaderMap::new();
        assert_eq!(client_ip(&empty, Some(ip(5)), true), Some(ip(5)));
    }

    #[test]
    fn trusted_skips_garbage_rightmost_hops() {
        // A trailing garbage hop must not nullify resolution; fall to the next.
        let headers = xff("1.1.1.1, not-an-ip");
        assert_eq!(
            client_ip(&headers, Some(ip(5)), true),
            Some(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)))
        );
    }
}
