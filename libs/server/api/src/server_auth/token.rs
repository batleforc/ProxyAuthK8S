use std::sync::Arc;

use axum::{
    Form,
    extract::{State, rejection::FormRejection},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tracing::instrument;
use utoipa::ToSchema;

use crate::cluster::auth::oauth::{
    Unredeemed, no_store, oauth_error, read_form, redeem_code, token::TokenErrorBody,
};
use crate::cluster::redirect::audit::{Event, HandlerAudit};
use crate::cluster::redirect::throttle;
use crate::server_auth::{CODE_PREFIX, SessionTokens, peer, throttled};

/// The form the CLI posts to redeem its single-use code.
#[derive(Deserialize, ToSchema)]
pub struct SessionTokenRequest {
    /// The single-use code `/auth/callback` sent to the loopback listener.
    pub code: String,
    /// The `redirect_uri` given to `/auth/login`.
    pub redirect_uri: String,
    /// The PKCE verifier of the `code_challenge` given to `/auth/login`.
    pub code_verifier: String,
}

/// Redeem the single-use code of the SSO session (RFC 0010)
///
/// The CLI trades the code `/auth/callback` sent to its loopback, with the
/// PKCE verifier, for the session. A code works once, whatever the outcome.
#[utoipa::path(
    post,
    path = "/auth/token",
    operation_id = "server_auth_token",
    tag = "auth_server",
    request_body(content = SessionTokenRequest, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "The session: the proxy token and its refresh token.", body = SessionTokens),
        (status = 400, description = "Invalid grant (`invalid_grant`): unknown, used or expired code, or a wrong verifier.", body = TokenErrorBody),
        (status = 429, description = "Rate limited or banned (see UNSCOPED_* environment knobs)."),
        (status = 503, description = "Redis is unavailable."),
    )
)]
#[instrument(name = "server_login_token", skip(req, data, form))]
pub async fn token(
    req: Parts,
    State(data): State<Arc<common::State>>,
    form: Result<Form<SessionTokenRequest>, FormRejection>,
) -> Response {
    let mut audit = HandlerAudit::new("", "");
    let response = token_inner(&req, &data, form, &mut audit).await;
    audit.finish(Event::ServerLogin, Event::ServerLogin, &response);
    response
}

async fn token_inner(
    req: &Parts,
    data: &common::State,
    form: Result<Form<SessionTokenRequest>, FormRejection>,
    audit: &mut HandlerAudit,
) -> Response {
    let form = match read_form(form) {
        Ok(form) => form,
        Err(response) => return *response,
    };
    let peer = peer(req);
    if let Some(response) = throttled(data, &peer).await {
        return response;
    }
    audit.redirect_uri(&form.redirect_uri);
    let key = format!("{CODE_PREFIX}:{}", form.code);
    let issued = match redeem_code(data, &key, &form.redirect_uri, &form.code_verifier, audit).await
    {
        Ok(issued) => issued,
        Err(Unredeemed::InvalidGrant) => {
            throttle::unscoped::record_auth_failure(data, &peer).await;
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
        }
        Err(Unredeemed::Unavailable) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    audit.grant(
        "authorization_code",
        issued
            .expires_in
            .and_then(|seconds| u64::try_from(seconds).ok()),
    );
    no_store(SessionTokens {
        access_token: issued.access_token,
        token_type: "Bearer".to_string(),
        expires_in: issued.expires_in,
        refresh_token: issued.refresh_token,
    })
}
