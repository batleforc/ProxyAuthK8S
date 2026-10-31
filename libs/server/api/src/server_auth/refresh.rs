use std::sync::Arc;

use axum::{
    Form,
    extract::{State, rejection::FormRejection},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use openidconnect::{OAuth2TokenResponse, RefreshToken, RequestTokenError};
use serde::Deserialize;
use tracing::{error, instrument, warn};
use utoipa::ToSchema;

use crate::cluster::redirect::audit::{Event, HandlerAudit};
use crate::cluster::redirect::throttle;
use crate::server_auth::{
    SessionError, SessionTokens, peer, session_conf, session_error, session_response, throttled,
};

#[derive(Deserialize, ToSchema)]
pub struct SessionRefreshRequest {
    /// The refresh token of the session; in the body, never in the URL.
    pub refresh_token: String,
}

/// Renew the SSO session to the proxy (RFC 0010)
///
/// The identity provider stays the only judge of the refresh token. A refused
/// one counts toward the unscoped ban; an unreachable provider does not.
#[utoipa::path(
    post,
    path = "/auth/refresh",
    operation_id = "server_auth_refresh",
    tag = "auth_server",
    request_body(content = SessionRefreshRequest, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "The renewed session.", body = SessionTokens),
        (status = 400, description = "invalid_grant: the identity provider refused the refresh token.", body = SessionError),
        (status = 429, description = "Rate limited or banned (see UNSCOPED_* environment knobs)."),
        (status = 503, description = "The identity provider is unavailable."),
    )
)]
#[instrument(name = "server_token_refresh", skip(req, data, form))]
pub async fn refresh(
    req: Parts,
    State(data): State<Arc<common::State>>,
    form: Result<Form<SessionRefreshRequest>, FormRejection>,
) -> Response {
    let mut audit = HandlerAudit::new("", "");
    let response = refresh_inner(&req, &data, form, &mut audit).await;
    audit.finish(Event::TokenRefreshed, Event::TokenRefreshed, &response);
    response
}

async fn refresh_inner(
    req: &Parts,
    data: &common::State,
    form: Result<Form<SessionRefreshRequest>, FormRejection>,
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
    let unavailable = |audit: &mut HandlerAudit| {
        audit.fail("the proxy's identity provider is unavailable");
        session_error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
    };
    let conf = session_conf(data);
    let (Ok(http), Ok(client)) = (conf.oidc_reqwest_client(), conf.oidc_core().await) else {
        return unavailable(audit);
    };
    let refresh_token = RefreshToken::new(form.refresh_token.clone());
    let Ok(request) = client.exchange_refresh_token(&refresh_token) else {
        return unavailable(audit);
    };
    match request.request_async(&http).await {
        Ok(tokens) => {
            let expires_in = tokens
                .expires_in()
                .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX));
            audit.grant(
                "refresh_token",
                expires_in.and_then(|s| u64::try_from(s).ok()),
            );
            throttle::unscoped::clear_auth_failures(data, &peer).await;
            session_response(SessionTokens {
                access_token: tokens.access_token().secret().clone(),
                token_type: "Bearer".to_string(),
                expires_in,
                // An IdP that does not rotate keeps the one presented.
                refresh_token: Some(tokens.refresh_token().map_or_else(
                    || form.refresh_token.clone(),
                    |token| token.secret().clone(),
                )),
            })
        }
        // The IdP judged the token: a fault of the caller's.
        Err(RequestTokenError::ServerResponse(response)) => {
            warn!(error = %response.error(), "the proxy's IdP refused a refresh token");
            audit.deny("invalid_grant");
            throttle::unscoped::record_auth_failure(data, &peer).await;
            session_error(StatusCode::BAD_REQUEST, "invalid_grant")
        }
        Err(e) => {
            error!(error = %e, "the refresh with the proxy's IdP failed");
            unavailable(audit)
        }
    }
}
