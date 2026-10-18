//! Request-handling helpers: the [`AuthError`] type, the `ns`/`cluster` path
//! parameters, and the readers of the bearer token and the peer address.

use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid token")]
    InvalidToken,
    #[error("Invalid token: {0}")]
    InvalidTokenDetail(String),
    #[error("No Authorization header found")]
    NoToken,
}

impl IntoResponse for AuthError {
    /// A plain-text `401` naming what is wrong with the credentials.
    fn into_response(self) -> Response {
        (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            self.to_string(),
        )
            .into_response()
    }
}

/// The `ns` and `cluster` path parameters shared by every cluster-scoped
/// route; the proxy route's `path` is read from the raw URI instead, never
/// through `Path` (which percent-decodes it).
#[derive(Debug, Deserialize)]
pub struct NsCluster {
    pub ns: String,
    pub cluster: String,
}

pub fn extract_authorization_header(headers: &HeaderMap) -> Result<&str, AuthError> {
    let raw = match headers.get(header::AUTHORIZATION) {
        // A non-ASCII header value is simply "invalid token"; do not echo the
        // raw bytes back to the caller.
        Some(value) => value.to_str().map_err(|_| AuthError::InvalidToken)?,
        None => return Err(AuthError::NoToken),
    };
    // The scheme is case-insensitive (RFC 6750/7235); the token itself is not.
    let token = match raw.split_once(' ') {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("Bearer") => rest,
        _ => return Err(AuthError::InvalidToken),
    };
    if token.trim().is_empty() {
        return Err(AuthError::InvalidToken);
    }
    Ok(token)
}

/// The address of the TCP peer, when the server recorded it (`ConnectInfo`);
/// `None` in a test that calls the router directly.
#[must_use]
pub fn peer_ip(parts: &Parts) -> Option<IpAddr> {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip())
}

/// The `x-forwarded-for` header, when it is valid ASCII.
#[must_use]
pub fn forwarded_for(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_auth(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        headers
    }

    #[test]
    fn extracts_a_bearer_token() {
        assert_eq!(
            extract_authorization_header(&with_auth("Bearer abc123")).unwrap(),
            "abc123"
        );
    }

    #[test]
    fn scheme_is_case_insensitive() {
        assert_eq!(
            extract_authorization_header(&with_auth("bearer abc")).unwrap(),
            "abc"
        );
        assert_eq!(
            extract_authorization_header(&with_auth("BEARER abc")).unwrap(),
            "abc"
        );
    }

    #[test]
    fn empty_token_is_rejected() {
        assert!(matches!(
            extract_authorization_header(&with_auth("Bearer ")),
            Err(AuthError::InvalidToken)
        ));
        assert!(matches!(
            extract_authorization_header(&with_auth("Bearer    ")),
            Err(AuthError::InvalidToken)
        ));
    }

    #[test]
    fn a_non_bearer_scheme_is_rejected() {
        assert!(matches!(
            extract_authorization_header(&with_auth("Basic abc")),
            Err(AuthError::InvalidToken)
        ));
    }

    #[test]
    fn a_missing_header_is_no_token() {
        assert!(matches!(
            extract_authorization_header(&HeaderMap::new()),
            Err(AuthError::NoToken)
        ));
    }

    #[test]
    fn the_401_names_the_problem() {
        let response = AuthError::NoToken.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
    }
}
