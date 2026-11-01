//! Mediated OAuth 2.0 Authorization Server (RFC 6749 + PKCE), per cluster.
//!
//! Advertised by [`crate::cluster::auth::well_known`]. The external caller
//! never needs a client registered with the cluster's upstream OIDC provider:
//! this proxy is that registered client, and mediates the whole exchange —
//! `/oauth/authorize` starts its own PKCE round-trip with the upstream
//! provider, `/oauth/callback` receives the upstream redirect and mints a
//! proxy-owned one-time code, and `/oauth/token` exchanges that code for the
//! upstream tokens once the external caller proves possession of its own PKCE
//! verifier.

pub mod authorize;
pub mod callback;
pub mod jwks;
pub mod model;
pub mod token;

use axum::{
    Form, Json,
    extract::rejection::FormRejection,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use reqwest::Url;
use serde::Serialize;
use tracing::error;

use crate::cluster::redirect::audit::HandlerAudit;
use model::{IssuedCode, redirect_uri_matches, verify_pkce_s256};
use token::TokenErrorBody;

/// Redirect to `redirect_uri` with an RFC 6749 §4.1.2.1 error response.
///
/// Used once `redirect_uri` itself has been validated: from that point on,
/// failures are reported to the external client via redirect rather than a
/// bare error body, so a CLI driving the flow can surface them.
pub(crate) fn redirect_with_error(
    redirect_uri: &Url,
    error: &str,
    state: Option<&str>,
) -> axum::response::Response {
    let mut url = redirect_uri.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("error", error);
        if let Some(state) = state {
            pairs.append_pair("state", state);
        }
    }
    found(url.as_str())
}

/// A `302 Found` to `location`.
pub(crate) fn found(location: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::FOUND,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response()
}

/// A form body as the token endpoints read it: over the limit a `413`, any
/// other unreadable form a `400` (axum's own answer would be a `422`).
/// The answer is boxed (clippy::result_large_err).
pub(crate) fn read_form<T>(form: Result<Form<T>, FormRejection>) -> Result<T, Box<Response>> {
    match form {
        Ok(Form(form)) => Ok(form),
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            Err(Box::new(StatusCode::PAYLOAD_TOO_LARGE.into_response()))
        }
        Err(rejection) => Err(Box::new(
            (StatusCode::BAD_REQUEST, rejection.body_text()).into_response(),
        )),
    }
}

/// An RFC 6749 §5.2 error body.
pub(crate) fn oauth_error(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(TokenErrorBody {
            error: error.to_string(),
        }),
    )
        .into_response()
}

/// A `200` carrying tokens, never cached (RFC 6749 §5.1).
pub(crate) fn no_store<T: Serialize>(body: T) -> Response {
    (
        StatusCode::OK,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(body),
    )
        .into_response()
}

/// Why a code was not redeemed.
pub(crate) enum Unredeemed {
    /// Unknown, spent, expired, or not this caller's: `invalid_grant`.
    InvalidGrant,
    /// The cache could not be read: `503`.
    Unavailable,
}

/// The issued code under `key`, if `redirect_uri` and `code_verifier` are the
/// ones it was issued for. Single-use: read and deleted at once whatever
/// follows, so a leaked, retried or replayed code is never redeemed twice.
/// The audit gets the subject and the reason of a refusal.
pub(crate) async fn redeem_code(
    data: &common::State,
    key: &str,
    redirect_uri: &str,
    code_verifier: &str,
    audit: &mut HandlerAudit,
) -> Result<IssuedCode, Unredeemed> {
    let raw = match data.redis_take(key).await {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            audit.deny("invalid_grant");
            return Err(Unredeemed::InvalidGrant);
        }
        Err(e) => {
            error!(error = %e, "couldn't get the issued code");
            return Err(Unredeemed::Unavailable);
        }
    };
    let issued = match serde_json::from_str::<IssuedCode>(&raw) {
        Ok(issued) => issued,
        Err(e) => {
            error!(error = %e, "couldn't parse the issued code");
            audit.deny("invalid_grant");
            return Err(Unredeemed::InvalidGrant);
        }
    };
    if !issued.subject.is_empty() {
        audit.user(&issued.subject, &[]);
    }
    if !redirect_uri_matches(redirect_uri, &issued.redirect_uri) {
        audit.deny("redirect_uri does not match the code");
        return Err(Unredeemed::InvalidGrant);
    }
    if !verify_pkce_s256(code_verifier, &issued.code_challenge) {
        audit.deny("PKCE verification failed");
        return Err(Unredeemed::InvalidGrant);
    }
    Ok(issued)
}
