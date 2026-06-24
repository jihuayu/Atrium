//! Pure helpers for issuing and reading HttpOnly auth cookies.
//!
//! Cookies are used as an optional, more secure transport for access/refresh
//! tokens. The JSON body still carries the tokens for non-browser clients, so
//! existing callers keep working unchanged.

use std::collections::HashMap;

pub const ACCESS_COOKIE: &str = "atrium_access";
pub const REFRESH_COOKIE: &str = "atrium_refresh";

/// Build a `Set-Cookie` header value.
///
/// `secure` should be `true` when the API is served over HTTPS so the cookie
/// is only ever sent over a TLS connection.
pub fn build_set_cookie(name: &str, value: &str, max_age_secs: i64, secure: bool) -> String {
    let mut value = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        name, value, max_age_secs
    );
    if secure {
        value.push_str("; Secure");
    }
    value
}

/// Build a `Set-Cookie` header value that immediately expires the cookie.
pub fn clear_cookie(name: &str, secure: bool) -> String {
    let mut value = format!("{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0", name);
    if secure {
        value.push_str("; Secure");
    }
    value
}

/// Parse a `Cookie` request header into a name→value map.
pub fn parse_cookies(header: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in header.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => (pair, ""),
        };
        out.insert(key.to_string(), value.to_string());
    }
    out
}

/// Extract a specific cookie value from a `Cookie` header.
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    for pair in header.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => (pair, ""),
        };
        if key == name {
            return Some(value);
        }
    }
    None
}

/// Returns `true` when `base_url` is an HTTPS URL, indicating cookies should
/// carry the `Secure` flag.
pub fn secure_from_base_url(base_url: &str) -> bool {
    base_url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::{build_set_cookie, clear_cookie, cookie_value, parse_cookies, secure_from_base_url};

    #[test]
    fn build_set_cookie_includes_attributes() {
        let v = build_set_cookie("atrium_access", "tok", 3600, true);
        assert!(v.contains("atrium_access=tok"));
        assert!(v.contains("HttpOnly"));
        assert!(v.contains("SameSite=Lax"));
        assert!(v.contains("Max-Age=3600"));
        assert!(v.contains("Secure"));
        assert!(v.contains("Path=/"));
    }

    #[test]
    fn build_set_cookie_omits_secure_when_http() {
        let v = build_set_cookie("atrium_access", "tok", 3600, false);
        assert!(!v.contains("Secure"));
    }

    #[test]
    fn clear_cookie_expires_immediately() {
        let v = clear_cookie("atrium_access", true);
        assert!(v.contains("atrium_access="));
        assert!(v.contains("Max-Age=0"));
        assert!(v.contains("Secure"));
    }

    #[test]
    fn parse_cookies_handles_multiple_pairs() {
        let map = parse_cookies("a=1; b=2; c=hello world");
        assert_eq!(map.get("a").map(String::as_str), Some("1"));
        assert_eq!(map.get("b").map(String::as_str), Some("2"));
        assert_eq!(map.get("c").map(String::as_str), Some("hello world"));
    }

    #[test]
    fn cookie_value_finds_named_cookie() {
        let header = "atrium_access=abc; other=xyz";
        assert_eq!(cookie_value(header, "atrium_access"), Some("abc"));
        assert_eq!(cookie_value(header, "missing"), None);
    }

    #[test]
    fn secure_from_base_url_detects_https() {
        assert!(secure_from_base_url("https://api.example.com"));
        assert!(!secure_from_base_url("http://localhost:3000"));
    }
}
