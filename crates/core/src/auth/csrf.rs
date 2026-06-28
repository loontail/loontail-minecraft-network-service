//! CSRF protection via the double-submit cookie pattern: a random token is set in a
//! readable cookie and echoed by the SPA in a header; a mutation is accepted only
//! when the two match. Complements the httpOnly session cookie, which a cross-site
//! form post cannot read.

use axum::http::HeaderMap;
use subtle::ConstantTimeEq;

use super::{cookie_value, generate_token};
use crate::error::{AppError, AppResult};

/// Cookie holding the CSRF token (readable by the SPA, not httpOnly).
pub const CSRF_COOKIE_NAME: &str = "loontail_csrf";
/// Request header the SPA echoes the CSRF token in.
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// Mint a fresh CSRF token (256-bit hex), to be set in the CSRF cookie at login.
pub fn generate_csrf_token() -> String {
    generate_token()
}

/// Constant-time byte-string equality, so token comparison leaks neither a matching
/// prefix nor the length through timing.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// Verify the double-submit invariant: the `x-csrf-token` header must be present,
/// non-empty, and byte-equal to the `loontail_csrf` cookie.
pub fn verify_csrf(headers: &HeaderMap) -> AppResult<()> {
    let cookie = cookie_value(headers, CSRF_COOKIE_NAME).ok_or(AppError::Forbidden)?;
    let header = headers
        .get(CSRF_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Forbidden)?;
    // why (SEC-7): constant-time compare so a near-miss token cannot be refined
    // byte-by-byte through response timing.
    if cookie.is_empty()
        || header.is_empty()
        || !constant_time_eq(cookie.as_bytes(), header.as_bytes())
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
