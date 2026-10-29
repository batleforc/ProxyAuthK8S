//! Kubernetes `Status` error bodies.
//!
//! The proxy sits where an apiserver is expected, so its own rejections must
//! look like apiserver rejections — otherwise `kubectl` prints a raw body
//! instead of a readable message.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

fn status_body(code: u16, reason: &str, message: &str) -> serde_json::Value {
    json!({
        "kind": "Status",
        "apiVersion": "v1",
        "metadata": {},
        "status": "Failure",
        "message": message,
        "reason": reason,
        "code": code,
    })
}

fn status_response(status: StatusCode, reason: &str, message: &str) -> Response {
    (status, Json(status_body(status.as_u16(), reason, message))).into_response()
}

/// 400 with a Kubernetes `Status` body.
#[must_use]
pub fn bad_request(message: &str) -> Response {
    status_response(StatusCode::BAD_REQUEST, "BadRequest", message)
}

/// 503 with a Kubernetes `Status` body: the cluster is reached through an
/// agent that has no tunnel open (RFC 0012). Never followed by a direct
/// connection.
#[must_use]
pub fn agent_unavailable() -> Response {
    status_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "ServiceUnavailable",
        "agent unavailable",
    )
}

/// 503 with a Kubernetes `Status` body naming who can act, for an upstream
/// mode this replica will not serve (RFC 0014 §4.2). Never followed by a
/// relay in another mode.
#[must_use]
pub fn upstream_mode_unavailable(message: &str) -> Response {
    status_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "ServiceUnavailable",
        message,
    )
}

/// 403 with a Kubernetes `Status` body.
#[must_use]
pub fn forbidden(message: &str) -> Response {
    status_response(StatusCode::FORBIDDEN, "Forbidden", message)
}

/// 401 with a Kubernetes `Status` body.
#[must_use]
pub fn unauthorized(message: &str) -> Response {
    status_response(StatusCode::UNAUTHORIZED, "Unauthorized", message)
}

/// 429 with a Kubernetes `Status` body and, when known, a `Retry-After` header.
///
/// `TooManyRequests` is the reason the apiserver itself uses when it throttles,
/// so `kubectl` already knows to back off on it.
#[must_use]
pub fn too_many_requests(message: &str, retry_after: Option<u64>) -> Response {
    let mut response = status_response(StatusCode::TOO_MANY_REQUESTS, "TooManyRequests", message);
    if let Some(retry_after) = retry_after {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_body_is_a_kubernetes_status() {
        let body = status_body(403, "Forbidden", "nope");
        assert_eq!(body["kind"], "Status");
        assert_eq!(body["apiVersion"], "v1");
        assert_eq!(body["status"], "Failure");
        assert_eq!(body["reason"], "Forbidden");
        assert_eq!(body["code"], 403);
        assert_eq!(body["message"], "nope");
    }

    #[test]
    fn responses_carry_the_expected_status_code() {
        assert_eq!(forbidden("nope").status().as_u16(), 403);
        assert_eq!(unauthorized("nope").status().as_u16(), 401);
        assert_eq!(too_many_requests("slow down", None).status().as_u16(), 429);
    }

    #[test]
    fn a_known_retry_delay_is_advertised() {
        let response = too_many_requests("slow down", Some(60));
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("60")
        );
        assert!(
            too_many_requests("slow down", None)
                .headers()
                .get("retry-after")
                .is_none()
        );
    }
}
