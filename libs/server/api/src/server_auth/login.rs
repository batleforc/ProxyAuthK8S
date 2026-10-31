use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use openidconnect::{CsrfToken, Nonce, PkceCodeChallenge, Scope, core::CoreAuthenticationFlow};
use serde::Deserialize;
use tracing::{error, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::oauth::{
    found,
    model::{
        PENDING_TTL_SECONDS, PendingAuthorization, is_valid_pkce_value, parse_loopback_redirect_uri,
    },
    redirect_with_error,
};
use crate::cluster::redirect::audit::{Event, HandlerAudit};
use crate::server_auth::{PENDING_PREFIX, peer, session_conf, throttled};

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct SessionLoginQuery {
    /// The CLI's loopback listener (`http://127.0.0.1:<port>/…`, `localhost`
    /// or `[::1]`); anything else is refused.
    pub redirect_uri: String,
    /// Opaque value echoed back verbatim on redirect.
    pub state: Option<String>,
    /// RFC 7636 `S256` challenge of the CLI's verifier.
    pub code_challenge: String,
}

/// Start the SSO session to the proxy (RFC 0010)
///
/// Redirects the browser to the proxy's identity provider; the CLI receives a
/// single-use code on its loopback `redirect_uri`, redeemed at `/auth/token`.
#[utoipa::path(
    get,
    path = "/auth/login",
    operation_id = "server_auth_login",
    tag = "auth_server",
    responses(
        (status = 302, description = "Redirect to the proxy's identity provider, or to redirect_uri with an error."),
        (status = 400, description = "redirect_uri is not a loopback URI."),
        (status = 429, description = "Rate limited or banned (see UNSCOPED_* environment knobs)."),
        (status = 500, description = "Internal server error."),
        (status = 503, description = "Redis is unavailable."),
    ),
    params(SessionLoginQuery)
)]
#[instrument(name = "server_login", skip(req, data, query))]
pub async fn login(
    req: Parts,
    State(data): State<Arc<common::State>>,
    query: Query<SessionLoginQuery>,
) -> Response {
    let mut audit = HandlerAudit::new("", "");
    let response = login_inner(&req, &data, &query, &mut audit).await;
    audit.finish(Event::ServerLogin, Event::ServerLogin, &response);
    response
}

async fn login_inner(
    req: &Parts,
    data: &common::State,
    query: &SessionLoginQuery,
    audit: &mut HandlerAudit,
) -> Response {
    if let Some(response) = throttled(data, &peer(req)).await {
        return response;
    }
    let Some(redirect_uri) = parse_loopback_redirect_uri(&query.redirect_uri) else {
        audit.deny("redirect_uri is not a loopback URI");
        return (
            StatusCode::BAD_REQUEST,
            "invalid_request: redirect_uri must be a loopback URI (http://localhost, http://127.0.0.1 or http://[::1])",
        )
            .into_response();
    };
    audit.redirect_uri(redirect_uri.as_str());
    if !is_valid_pkce_value(&query.code_challenge) {
        return redirect_with_error(&redirect_uri, "invalid_request", query.state.as_deref());
    }

    let conf = session_conf(data);
    let client = match conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get the proxy's oidc client");
            return redirect_with_error(&redirect_uri, "server_error", query.state.as_deref());
        }
    };
    let correlation_id = CsrfToken::new_random().secret().clone();
    let nonce_value = Nonce::new_random().secret().clone();
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let scopes: Vec<Scope> = conf
        .scopes
        .split_whitespace()
        .map(|s| Scope::new(s.to_string()))
        .collect();
    let (auth_url, _csrf, _nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            {
                let correlation_id = correlation_id.clone();
                move || CsrfToken::new(correlation_id)
            },
            {
                let nonce_value = nonce_value.clone();
                move || Nonce::new(nonce_value)
            },
        )
        .set_pkce_challenge(pkce_challenge)
        .add_scopes(scopes)
        .url();

    let pending = PendingAuthorization {
        client_redirect_uri: redirect_uri.to_string(),
        client_state: query.state.clone(),
        client_code_challenge: query.code_challenge.clone(),
        upstream_nonce: nonce_value,
        upstream_pkce_verifier: pkce_verifier.secret().clone(),
    };
    let Ok(pending_json) = serde_json::to_string(&pending) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if let Err(e) = data
        .redis_set(
            &format!("{PENDING_PREFIX}:{correlation_id}"),
            &pending_json,
            Some(PENDING_TTL_SECONDS),
        )
        .await
    {
        error!(error = %e, "couldn't store the pending login");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    found(auth_url.as_str())
}
