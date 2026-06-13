//! CSRF protection for admin mutations using the double-submit cookie pattern:
//! a random token is set in a readable cookie and echoed by the SPA in a request
//! header. A mutation is accepted only when the two match. This complements the
//! httpOnly session cookie (which a cross-site form post cannot read).

use axum::http::header::COOKIE;
use axum::http::HeaderMap;

use super::generate_token;
use crate::error::{AppError, AppResult};

/// Cookie holding the CSRF token (readable by the SPA, not httpOnly).
pub const CSRF_COOKIE_NAME: &str = "loontail_csrf";
/// Request header the SPA echoes the CSRF token in.
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// Mint a fresh CSRF token (256-bit hex), to be set in the CSRF cookie at login.
pub fn generate_csrf_token() -> String {
    generate_token()
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim())
    })
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
    if cookie.is_empty() || header.is_empty() || cookie != header {
        return Err(AppError::Forbidden);
    }
    Ok(())
}
