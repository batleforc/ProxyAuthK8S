//! The SSO session to the proxy itself (RFC 0010 §6.5), for the CLI of a
//! `TokenExchange` cluster: one login, one refresh token, every cluster.
//!
//! The same mediated flow as the per-cluster OAuth server
//! ([`crate::cluster::auth::oauth`]), on the proxy's own IdP and client
//! (`OIDC_*`): `/auth/login` sends the browser to the IdP, which calls back
//! the proxy at `/auth/callback`; the proxy exchanges the code, mints a
//! single-use code for the CLI's loopback listener, and `/auth/token` hands
//! the tokens over to whoever holds the PKCE verifier. Tokens never travel in
//! a URL. `/auth/refresh` renews the pair with the IdP.
//!
//! These routes have no cluster, so they use the unscoped budget
//! (`UNSCOPED_*`, see [`crate::cluster::redirect::throttle::unscoped`]).

pub mod callback;
pub mod login;
pub mod refresh;
pub mod token;

use axum::{http::request::Parts, response::Response};
use common::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    cluster::redirect::{forwarded, status_response::too_many_requests, throttle},
    helper::{forwarded_for, peer_ip},
};

/// Redis key prefix of the pending logins, keyed by the IdP `state`.
pub const PENDING_PREFIX: &str = "server_auth_pending";
/// Redis key prefix of the codes handed to the CLI.
pub const CODE_PREFIX: &str = "server_auth_code";
/// The path the IdP calls back, under the proxy's public URL.
pub const CALLBACK_PATH: &str = "/api/v1/auth/callback";
/// The largest form `/auth/token` and `/auth/refresh` read.
pub const FORM_LIMIT: usize = 16 * 1024;

/// The proxy's own OIDC client, redirecting to [`CALLBACK_PATH`].
pub(crate) fn session_conf(data: &State) -> common::oidc_conf::OidcConf {
    let mut conf = data.oidc_client.clone();
    conf.redirect_url = Some(format!(
        "{}{CALLBACK_PATH}",
        data.oidc_cluster_redirect_base_url.trim_end_matches('/')
    ));
    conf
}

/// The caller's address, for the unscoped budget.
pub(crate) fn peer(req: &Parts) -> String {
    forwarded::throttle_client_ip(forwarded_for(&req.headers), peer_ip(req))
}

/// The ban and rate-limit checks of the unscoped budget, before any call to
/// the IdP (T12).
pub(crate) async fn throttled(data: &State, peer: &str) -> Option<Response> {
    if throttle::unscoped::is_banned(data, peer).await {
        let retry_after = throttle::unscoped::ban_retry_after(data, peer).await;
        let throttled = throttle::Throttled::Banned { retry_after };
        tracing::warn!(%peer, "refusing a banned client on the session surface");
        return Some(too_many_requests(
            &throttled.message(),
            throttled.retry_after(),
        ));
    }
    throttle::unscoped::check_rate_limit(data, peer)
        .await
        .map(|throttled| too_many_requests(&throttled.message(), throttled.retry_after()))
}

/// The session the CLI keeps: the proxy token and what renews it.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SessionTokens {
    pub access_token: String,
    pub token_type: String,
    /// Seconds until `access_token` expires, when the IdP said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    /// Absent when the IdP issued none (no session to renew).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// An OAuth error body (`invalid_grant`, `invalid_request`…).
#[derive(Serialize, ToSchema)]
pub struct SessionError {
    pub error: String,
}

pub(crate) fn session_error(status: axum::http::StatusCode, error: &str) -> Response {
    use axum::response::IntoResponse;
    (
        status,
        axum::Json(SessionError {
            error: error.to_string(),
        }),
    )
        .into_response()
}

/// A `200` carrying the session, never cached.
pub(crate) fn session_response(tokens: SessionTokens) -> Response {
    use axum::{http::header, response::IntoResponse};
    (
        axum::http::StatusCode::OK,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        axum::Json(tokens),
    )
        .into_response()
}
