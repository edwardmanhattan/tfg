//! Bearer-token plumbing for the control plane.
//!
//! Two dull things: read the `Authorization` header, and compare two byte
//! strings without leaking where they stopped matching. Deciding *whose*
//! credential a token is — the operator's, a device's, nobody's — is
//! `devices::Credentials`, because there is exactly one place that answer is
//! given and a second one would be a second answer.

use hyper::HeaderMap;
use hyper::header::AUTHORIZATION;

/// The scheme prefix, case-insensitive per RFC 7235.
const BEARER: &str = "bearer ";

/// The shortest token the server will start with: 32 characters, which is 256
/// bits when the token is random as it should be. A short token is a typo or a
/// placeholder, and either way it is not a secret.
pub const MIN_TOKEN_LEN: usize = 32;

/// The `Authorization: Bearer …` value, if the request carries one.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let scheme = value.get(..BEARER.len())?;
    if !scheme.eq_ignore_ascii_case(BEARER) {
        return None;
    }
    let token = value[BEARER.len()..].trim();
    if token.is_empty() { None } else { Some(token) }
}

/// Compare without an early exit. Length is compared first because the loop
/// below walks the longer of the two; the length difference still contributes
/// to the result, so a wrong-length token cannot pass.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(left.get(index).copied().unwrap_or(0))
            ^ usize::from(right.get(index).copied().unwrap_or(0));
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::HeaderValue;

    fn headers(value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(
            AUTHORIZATION,
            HeaderValue::from_str(value).expect("header value"),
        );
        map
    }

    #[test]
    fn a_bearer_token_is_read_back_without_its_scheme() {
        assert_eq!(bearer(&headers("Bearer abc123")), Some("abc123"));
        assert_eq!(bearer(&headers("bearer abc123")), Some("abc123"));
        assert_eq!(bearer(&headers("Basic abc123")), None);
        assert_eq!(bearer(&headers("Bearer ")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }

    #[test]
    fn comparison_checks_length_and_contents() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
