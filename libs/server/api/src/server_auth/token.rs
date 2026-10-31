use std::sync::Arc;

use axum::{
    Form,
    extract::{State, rejection::FormRejection},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tracing::{error, instrument};
use utoipa::ToSchema;

use crate::cluster::auth::oauth::model::{IssuedCode, redirect_uri_matches, verify_pkce_s256};
use crate::cluster::redirect::audit::{Event, HandlerAudit};
use crate::cluster::redirect::throttle;
use crate::server_auth::{
    CODE_PREFIX, SessionError, SessionTokens, peer, session_error, session_response, throttled,
};

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
        (status = 400, description = "Invalid grant (`invalid_grant`): unknown, used or expired code, or a wrong verifier.", body = SessionError),
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
    let form = match form {
        Ok(form) => form,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        Err(rejection) => return (StatusCode::BAD_REQUEST, rejection.body_text()).into_response(),
    };
    let peer = peer(req);
    if let Some(response) = throttled(data, &peer).await {
        return response;
    }
    audit.redirect_uri(&form.redirect_uri);
    // Single-use whatever follows: a replayed code finds nothing.
    let issued = match data
        .redis_take(&format!("{CODE_PREFIX}:{}", form.code))
        .await
    {
        Ok(Some(raw)) => serde_json::from_str::<IssuedCode>(&raw).ok(),
        Ok(None) => None,
        Err(e) => {
            error!(error = %e, "couldn't get the issued code");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let Some(issued) = issued.filter(|issued| {
        redirect_uri_matches(&form.redirect_uri, &issued.redirect_uri)
            && verify_pkce_s256(&form.code_verifier, &issued.code_challenge)
    }) else {
        audit.deny("invalid_grant");
        throttle::unscoped::record_auth_failure(data, &peer).await;
        return session_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    audit.user(&issued.subject, &[]);
    audit.grant(
        "authorization_code",
        issued
            .expires_in
            .and_then(|seconds| u64::try_from(seconds).ok()),
    );
    session_response(SessionTokens {
        access_token: issued.access_token,
        token_type: "Bearer".to_string(),
        expires_in: issued.expires_in,
        refresh_token: issued.refresh_token,
    })
}
