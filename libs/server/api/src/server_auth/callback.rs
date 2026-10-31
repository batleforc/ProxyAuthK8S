use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, Nonce, OAuth2TokenResponse, PkceCodeVerifier, TokenResponse,
};
use reqwest::Url;
use serde::Deserialize;
use tracing::{error, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::oauth::{
    found,
    model::{CODE_TTL_SECONDS, IssuedCode, PendingAuthorization},
    redirect_with_error,
};
use crate::cluster::redirect::audit::{Event, HandlerAudit};
use crate::server_auth::{CODE_PREFIX, PENDING_PREFIX, peer, session_conf, throttled};

/// The identity provider's callback query.
#[derive(Deserialize, ToSchema, IntoParams)]
pub struct SessionCallbackQuery {
    /// Authorization code from the proxy's identity provider; absent when it
    /// answers with an `error` instead.
    pub code: Option<String>,
    /// The identity provider's error (`access_denied`…), sent back to the CLI
    /// as `access_denied`.
    pub error: Option<String>,
    /// The correlation id this proxy generated at `/auth/login`.
    pub state: String,
}

/// Callback from the proxy's identity provider, for the SSO session (RFC 0010)
///
/// Not meant to be opened directly. On success, redirects to the CLI's
/// loopback `redirect_uri` with a single-use code, never a token.
#[utoipa::path(
    get,
    path = "/auth/callback",
    operation_id = "server_auth_callback",
    tag = "auth_server",
    responses(
        (status = 302, description = "Redirect to the CLI's redirect_uri, with a single-use code (or an error)."),
        (status = 400, description = "Unknown or expired state."),
        (status = 429, description = "Rate limited or banned (see UNSCOPED_* environment knobs)."),
        (status = 500, description = "Internal server error."),
        (status = 503, description = "Redis is unavailable."),
    ),
    params(SessionCallbackQuery)
)]
#[instrument(name = "server_login_callback", skip(req, data, callback))]
pub async fn callback(
    req: Parts,
    State(data): State<Arc<common::State>>,
    callback: Query<SessionCallbackQuery>,
) -> Response {
    let mut audit = HandlerAudit::new("", "");
    let response = callback_inner(&req, &data, &callback, &mut audit).await;
    audit.finish(Event::ServerLogin, Event::ServerLogin, &response);
    response
}

async fn callback_inner(
    req: &Parts,
    data: &common::State,
    callback: &SessionCallbackQuery,
    audit: &mut HandlerAudit,
) -> Response {
    if let Some(response) = throttled(data, &peer(req)).await {
        return response;
    }
    // Single-use: read and deleted at once, so a replayed callback finds
    // nothing.
    let pending = match data
        .redis_take(&format!("{PENDING_PREFIX}:{}", callback.state))
        .await
    {
        Ok(Some(raw)) => match serde_json::from_str::<PendingAuthorization>(&raw) {
            Ok(pending) => pending,
            Err(_) => {
                audit.deny("invalid or expired state");
                return (StatusCode::BAD_REQUEST, "invalid state").into_response();
            }
        },
        Ok(None) => {
            audit.deny("invalid or expired state");
            return (StatusCode::BAD_REQUEST, "invalid or expired state").into_response();
        }
        Err(e) => {
            error!(error = %e, "couldn't get the pending login");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let Ok(redirect_uri) = Url::parse(&pending.client_redirect_uri) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    audit.redirect_uri(redirect_uri.as_str());
    let state = pending.client_state.as_deref();
    let fail = |error: &str| redirect_with_error(&redirect_uri, error, state);
    // The user cancelled or was refused at the IdP: the CLI hears it now
    // instead of waiting for its timeout.
    let Some(code) = callback.code.clone().filter(|_| callback.error.is_none()) else {
        audit.deny("the identity provider answered with an error");
        return fail("access_denied");
    };

    let conf = session_conf(data);
    let (Ok(http), Ok(client)) = (conf.oidc_reqwest_client(), conf.oidc_core().await) else {
        error!("couldn't build the proxy's oidc client");
        return fail("server_error");
    };
    let Ok(exchange) = client.exchange_code(AuthorizationCode::new(code)) else {
        return fail("server_error");
    };
    let tokens = match exchange
        .set_pkce_verifier(PkceCodeVerifier::new(pending.upstream_pkce_verifier))
        .request_async(&http)
        .await
    {
        Ok(tokens) => tokens,
        Err(e) => {
            error!(error = %e, "the code exchange with the proxy's IdP failed");
            return fail("access_denied");
        }
    };
    let Some(id_token) = tokens.id_token() else {
        return fail("server_error");
    };
    let verifier = client.id_token_verifier();
    let claims = match id_token.claims(&verifier, &Nonce::new(pending.upstream_nonce)) {
        Ok(claims) => claims,
        Err(e) => {
            error!(error = %e, "couldn't verify the ID token");
            return fail("server_error");
        }
    };
    if let Some(expected) = claims.access_token_hash() {
        let (Ok(alg), Ok(key)) = (id_token.signing_alg(), id_token.signing_key(&verifier)) else {
            return fail("server_error");
        };
        match AccessTokenHash::from_token(tokens.access_token(), alg, key) {
            Ok(actual) if actual == *expected => {}
            Ok(_) => return fail("access_denied"),
            Err(_) => return fail("server_error"),
        }
    }
    audit.user(claims.subject().as_str(), &[]);

    let code = openidconnect::CsrfToken::new_random().secret().clone();
    let issued = IssuedCode {
        subject: claims.subject().to_string(),
        redirect_uri: pending.client_redirect_uri.clone(),
        code_challenge: pending.client_code_challenge,
        access_token: tokens.access_token().secret().clone(),
        refresh_token: tokens.refresh_token().map(|token| token.secret().clone()),
        id_token: id_token.to_string(),
        scope: conf.scopes.clone(),
        expires_in: tokens
            .expires_in()
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)),
    };
    let Ok(issued_json) = serde_json::to_string(&issued) else {
        return fail("server_error");
    };
    if let Err(e) = data
        .redis_set(
            &format!("{CODE_PREFIX}:{code}"),
            &issued_json,
            Some(CODE_TTL_SECONDS),
        )
        .await
    {
        error!(error = %e, "couldn't store the issued code");
        return fail("server_error");
    }
    let mut location = redirect_uri.clone();
    {
        let mut pairs = location.query_pairs_mut();
        pairs.append_pair("code", &code);
        if let Some(state) = state {
            pairs.append_pair("state", state);
        }
    }
    found(location.as_str())
}
