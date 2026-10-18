use crate::cluster::redirect::audit::{Event, HandlerAudit};
use std::sync::Arc;

use axum::{
    Form, Json,
    extract::{Path, State, rejection::FormRejection},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tracing::{error, instrument};
use utoipa::ToSchema;

use crate::cluster::auth::{
    load_discovery_enabled_proxy,
    oauth::model::{CODE_PREFIX, IssuedCode, redirect_uri_matches, verify_pkce_s256},
    throttle_oauth_as,
};
use crate::helper::NsCluster;

#[derive(Deserialize, ToSchema)]
pub struct TokenRequest {
    /// Must be `authorization_code`; this server only implements that grant.
    pub grant_type: String,
    /// The proxy-minted code returned by `/oauth/callback`.
    pub code: String,
    /// Must match the `redirect_uri` presented at `/oauth/authorize` (RFC 6749 §4.1.3).
    pub redirect_uri: String,
    /// RFC 7636 PKCE verifier for the `code_challenge` presented at `/oauth/authorize`.
    pub code_verifier: String,
}

#[derive(Serialize, ToSchema)]
pub struct TokenResponseBody {
    pub access_token: String,
    pub token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub id_token: String,
    pub scope: String,
}

#[derive(Serialize, ToSchema)]
pub struct TokenErrorBody {
    pub error: String,
}

fn token_error(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(TokenErrorBody {
            error: error.to_string(),
        }),
    )
        .into_response()
}

/// The largest form body the token endpoint reads: actix's `Form` default,
/// kept by the move to axum (RFC 0007 contract).
pub const TOKEN_FORM_LIMIT: usize = 16 * 1024;

/// Exchange a proxy-minted authorization code for the upstream tokens
///
/// RFC 6749 §4.1.3 token endpoint. If the cluster is not found, disabled, or
/// discovery is not enabled, return 404.
#[utoipa::path(
    post,
    path = "/{ns}/{cluster}/oauth/token",
    tag = "auth_clusters",
    request_body(content = TokenRequest, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Token response.", body = TokenResponseBody),
        (status = 400, description = "invalid_request / invalid_grant / unsupported_grant_type.", body = TokenErrorBody),
        (status = 404, description = "Cluster not found, disabled, or discovery not enabled."),
        (status = 503, description = "Cache unreachable."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
    )
)]
#[instrument(name = "oauth_as_token", skip(req, data, form))]
pub async fn token(
    req: Parts,
    State(data): State<Arc<common::State>>,
    Path(NsCluster { ns, cluster }): Path<NsCluster>,
    form: Result<Form<TokenRequest>, FormRejection>,
) -> Response {
    let mut audit = HandlerAudit::new(&ns, &cluster);
    let response = token_inner(req, data, ns, cluster, form, &mut audit).await;
    audit.finish(Event::OauthTokenIssued, Event::OauthTokenDenied, &response);
    response
}

/// The handler itself; [`token`] writes its audit line.
async fn token_inner(
    req: Parts,
    data: Arc<common::State>,
    ns: String,
    cluster: String,
    form: Result<Form<TokenRequest>, FormRejection>,
    audit: &mut HandlerAudit,
) -> Response {
    // As under actix: a body over the limit is a 413, any other unreadable
    // form a 400 (axum's own answer would be a 422).
    let form = match form {
        Ok(form) => form,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        Err(rejection) => return (StatusCode::BAD_REQUEST, rejection.body_text()).into_response(),
    };
    let proxy = match load_discovery_enabled_proxy(&data, &ns, &cluster).await {
        Ok(proxy) => proxy,
        Err(gate) => return gate.into_response(),
    };
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }

    audit.redirect_uri(&form.redirect_uri);
    if form.grant_type != "authorization_code" {
        audit.deny("unsupported_grant_type");
        return token_error(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }

    let code_key = format!("{CODE_PREFIX}:{ns}/{cluster}/{}", form.code);
    // Single-use: the code is read and deleted atomically, regardless of what
    // follows, so a leaked, retried or concurrently replayed code can never be
    // redeemed twice.
    let issued = match data.redis_take(&code_key).await {
        Ok(Some(raw)) => match serde_json::from_str::<IssuedCode>(&raw) {
            Ok(issued) => issued,
            Err(e) => {
                error!(error = %e, "couldn't parse issued code");
                audit.deny("invalid_grant");
                return token_error(StatusCode::BAD_REQUEST, "invalid_grant");
            }
        },
        Ok(None) => {
            audit.deny("invalid_grant");
            return token_error(StatusCode::BAD_REQUEST, "invalid_grant");
        }
        Err(e) => {
            error!(error = %e, "couldn't get issued code");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    if !issued.subject.is_empty() {
        audit.user(&issued.subject, &[]);
    }
    if !redirect_uri_matches(&form.redirect_uri, &issued.redirect_uri) {
        audit.deny("redirect_uri does not match the code");
        return token_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    if !verify_pkce_s256(&form.code_verifier, &issued.code_challenge) {
        audit.deny("PKCE verification failed");
        return token_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }

    audit.grant(
        "authorization_code",
        issued
            .expires_in
            .and_then(|seconds| u64::try_from(seconds).ok()),
    );
    (
        StatusCode::OK,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(TokenResponseBody {
            access_token: issued.access_token,
            token_type: "Bearer".to_string(),
            expires_in: issued.expires_in,
            refresh_token: issued.refresh_token,
            id_token: issued.id_token,
            scope: issued.scope,
        }),
    )
        .into_response()
}
