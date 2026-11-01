use crate::cluster::redirect::audit::{Event, HandlerAudit};
use std::sync::Arc;

use axum::{
    Form,
    extract::{Path, State, rejection::FormRejection},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use utoipa::ToSchema;

use crate::cluster::auth::{
    load_discovery_enabled_proxy,
    oauth::{Unredeemed, model::CODE_PREFIX, no_store, oauth_error, read_form, redeem_code},
    throttle_oauth_as,
};
use crate::helper::NsCluster;

/// The token request, form-encoded (RFC 6749 §4.1.3 and §6).
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

/// A successful token response (RFC 6749 §5.1).
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

/// An OAuth error response (RFC 6749 §5.2).
#[derive(Serialize, ToSchema)]
pub struct TokenErrorBody {
    pub error: String,
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
        (status = 200, description = "The tokens for the cluster: access, ID and refresh.", body = TokenResponseBody),
        (status = 400, description = "OAuth error: `invalid_request`, `invalid_grant` or `unsupported_grant_type`.", body = TokenErrorBody),
        (status = 404, description = "Cluster not found, disabled, or discovery not enabled."),
        (status = 503, description = "The cache (Redis) is unreachable."),
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
    let form = match read_form(form) {
        Ok(form) => form,
        Err(response) => return *response,
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
        return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }

    let code_key = format!("{CODE_PREFIX}:{ns}/{cluster}/{}", form.code);
    let issued = match redeem_code(
        &data,
        &code_key,
        &form.redirect_uri,
        &form.code_verifier,
        audit,
    )
    .await
    {
        Ok(issued) => issued,
        Err(Unredeemed::InvalidGrant) => {
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
    no_store(TokenResponseBody {
        access_token: issued.access_token,
        token_type: "Bearer".to_string(),
        expires_in: issued.expires_in,
        refresh_token: issued.refresh_token,
        id_token: issued.id_token,
        scope: issued.scope,
    })
}
