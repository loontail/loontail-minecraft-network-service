//! CSRF protection for admin mutations using the double-submit cookie pattern:
//! a random token is set in a readable cookie and echoed by the SPA in a request
//! header. A mutation is accepted only when the two match. This complements the
//! httpOnly session cookie (which a cross-site form post cannot read).

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

/// Constant-time equality over two byte strings. Avoids leaking a match prefix
/// through timing on token comparisons. Lengths are compared in constant time too
/// (`ConstantTimeEq` for slices folds the length into the result).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// Verify the double-submit invariant: the `x-csrf-token` header must be present,
/// non-empty, and byte-equal to the `loontail_csrf` cookie. Returns
/// `AppError::Forbidden` on any mismatch. Call this in admin mutation handlers.
pub fn verify_csrf(headers: &HeaderMap) -> AppResult<()> {
    let cookie = cookie_value(headers, CSRF_COOKIE_NAME).ok_or(AppError::Forbidden)?;
    let header = headers
        .get(CSRF_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Forbidden)?;
    // why: constant-time compare so a near-miss token cannot be refined byte-by-byte
    // through response timing (SEC-7).
    if cookie.is_empty()
        || header.is_empty()
        || !constant_time_eq(cookie.as_bytes(), header.as_bytes())
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
