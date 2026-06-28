//! Cookie builders for the admin session + CSRF tokens. The session cookie is
//! httpOnly; the CSRF cookie is deliberately readable so the SPA can echo it in
//! the `x-csrf-token` header (double-submit pattern).

use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use chrono::{DateTime, Utc};

use loontail_core::auth::CSRF_COOKIE_NAME;

/// Build a `Set-Cookie` value; `max_age` of `None` clears the cookie.
fn build_cookie(name: &str, value: &str, http_only: bool, max_age: Option<i64>) -> String {
    let mut cookie = format!("{name}={value}; Path=/; Secure; SameSite=Lax");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    match max_age {
        Some(seconds) => cookie.push_str(&format!("; Max-Age={seconds}")),
        // why: Max-Age=0 plus a past Expires reliably evicts the cookie across browsers.
        None => cookie.push_str("; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT"),
    }
    cookie
}

fn append(headers: &mut HeaderMap, cookie: String) {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        headers.append(SET_COOKIE, value);
    }
}

pub fn set_session_cookies(
    headers: &mut HeaderMap,
    cookie_name: &str,
    session_token: &str,
    csrf_token: &str,
    expires_at: DateTime<Utc>,
) {
    let max_age = (expires_at - Utc::now()).num_seconds().max(0);
    append(
        headers,
        build_cookie(cookie_name, session_token, true, Some(max_age)),
    );
    append(
        headers,
        build_cookie(CSRF_COOKIE_NAME, csrf_token, false, Some(max_age)),
    );
}

pub fn clear_session_cookies(headers: &mut HeaderMap, cookie_name: &str) {
    append(headers, build_cookie(cookie_name, "", true, None));
    append(headers, build_cookie(CSRF_COOKIE_NAME, "", false, None));
}

/// Read a named cookie value (used to find the raw session token on logout,
/// where there is no extractor to lean on).
pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    loontail_core::cookie_value(headers, name).map(str::to_string)
}
