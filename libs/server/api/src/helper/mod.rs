//! Request-handling helpers: the [`AuthError`] type and the extractors that
//! pull the bearer token and the namespace/cluster pair out of a request.

use actix_web::{HttpRequest, HttpResponse, http::header::ContentType};
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

impl AuthError {
    #[must_use]
    pub fn into_http_response(&self) -> HttpResponse {
        match self {
            AuthError::InvalidToken => HttpResponse::Unauthorized()
                .content_type(ContentType::plaintext())
                .body("Invalid token"),
            AuthError::NoToken => HttpResponse::Unauthorized()
                .content_type(ContentType::plaintext())
                .body("No Authorization header found"),
            AuthError::InvalidTokenDetail(detail) => HttpResponse::Unauthorized()
                .content_type(ContentType::plaintext())
                .body(format!("Invalid token: {detail}")),
        }
    }

    #[must_use]
    pub fn into_actix_error(&self) -> actix_web::Error {
        match self {
            AuthError::InvalidToken => actix_web::error::ErrorUnauthorized("Invalid token"),
            AuthError::NoToken => {
                actix_web::error::ErrorUnauthorized("No Authorization header found")
            }
            AuthError::InvalidTokenDetail(detail) => {
                actix_web::error::ErrorUnauthorized(format!("Invalid token: {detail}"))
            }
        }
    }
}

/// Read the `ns` and `cluster` path parameters shared by every cluster-scoped route.
///
/// The routes always declare both, but reading them without unwrapping keeps a
/// mis-registered route from panicking a worker.
#[must_use]
pub fn extract_ns_cluster(req: &HttpRequest) -> Option<(String, String)> {
    let match_info = req.match_info();
    match (match_info.get("ns"), match_info.get("cluster")) {
        (Some(ns), Some(cluster)) => Some((ns.to_string(), cluster.to_string())),
        _ => None,
    }
}

pub fn extract_authorization_header(req: &HttpRequest) -> Result<&str, AuthError> {
    let raw = match req.headers().get("Authorization") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    fn with_auth(value: &str) -> actix_web::HttpRequest {
        TestRequest::default()
            .insert_header(("Authorization", value))
            .to_http_request()
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
        let req = TestRequest::default().to_http_request();
        assert!(matches!(
            extract_authorization_header(&req),
            Err(AuthError::NoToken)
        ));
    }
}
