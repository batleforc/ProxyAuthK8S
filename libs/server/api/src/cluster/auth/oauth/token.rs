use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use common::State;
use serde::{Deserialize, Serialize};
use tracing::{error, instrument};
use utoipa::ToSchema;

use crate::cluster::auth::{
    load_discovery_enabled_proxy,
    oauth::model::{CODE_PREFIX, IssuedCode, redirect_uri_matches, verify_pkce_s256},
    throttle_oauth_as,
};
use crate::helper::extract_ns_cluster;

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

fn token_error(status: actix_web::http::StatusCode, error: &str) -> HttpResponse {
    HttpResponse::build(status).json(TokenErrorBody {
        error: error.to_string(),
    })
}

/// Exchange a proxy-minted authorization code for the upstream tokens
///
/// RFC 6749 §4.1.3 token endpoint. If the cluster is not found, disabled, or
/// discovery is not enabled, return 404.
#[utoipa::path(
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
#[post("/{ns}/{cluster}/oauth/token")]
#[instrument(name = "oauth_as_token", skip(data, form))]
pub async fn token(
    req: HttpRequest,
    data: web::Data<State>,
    form: web::Form<TokenRequest>,
) -> impl Responder {
    let (ns, cluster) = if let Some(parts) = extract_ns_cluster(&req) {
        parts
    } else {
        error!(path = %req.path(), "missing ns/cluster path parameters");
        return HttpResponse::NotFound().finish();
    };
    let proxy = match load_discovery_enabled_proxy(&data, &ns, &cluster).await {
        Ok(proxy) => proxy,
        Err(gate) => return gate.into_response(),
    };
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }

    if form.grant_type != "authorization_code" {
        return token_error(
            actix_web::http::StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
        );
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
                return token_error(actix_web::http::StatusCode::BAD_REQUEST, "invalid_grant");
            }
        },
        Ok(None) => return token_error(actix_web::http::StatusCode::BAD_REQUEST, "invalid_grant"),
        Err(e) => {
            error!(error = %e, "couldn't get issued code");
            return HttpResponse::ServiceUnavailable().finish();
        }
    };
    if !redirect_uri_matches(&form.redirect_uri, &issued.redirect_uri) {
        return token_error(actix_web::http::StatusCode::BAD_REQUEST, "invalid_grant");
    }
    if !verify_pkce_s256(&form.code_verifier, &issued.code_challenge) {
        return token_error(actix_web::http::StatusCode::BAD_REQUEST, "invalid_grant");
    }

    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .insert_header(("Pragma", "no-cache"))
        .json(TokenResponseBody {
            access_token: issued.access_token,
            token_type: "Bearer".to_string(),
            expires_in: issued.expires_in,
            refresh_token: issued.refresh_token,
            id_token: issued.id_token,
            scope: issued.scope,
        })
}
