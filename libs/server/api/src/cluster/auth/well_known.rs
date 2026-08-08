use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd_runtime::ProxyKubeApiRuntime;
use serde::{Deserialize, Serialize};
use tracing::{error, instrument};
use utoipa::ToSchema;

use crate::cluster::auth::{load_discovery_enabled_proxy, throttle_oauth_as};
use crate::helper::extract_ns_cluster;

/// OAuth 2.0 Authorization Server Metadata (RFC 8414) for a proxied cluster.
///
/// `issuer` is this proxy's own cluster URL, and so are every other endpoint:
/// this proxy is a full mediating Authorization Server (see
/// [`crate::cluster::auth::oauth`]), not a pointer to the cluster's upstream
/// OIDC provider. A caller never needs to be registered with — or even learn
/// the hostname of — the upstream provider; the proxy is.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct OAuthAuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
}

/// OAuth 2.0 Authorization Server Metadata for the cluster's mediated OAuth Authorization Server
///
/// Unauthenticated by nature (RFC 8414 discovery). If the cluster is not
/// found, disabled, or does not have discovery enabled, return 404.
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 200, description = "OAuth 2.0 authorization server metadata (RFC 8414).", body = OAuthAuthorizationServerMetadata),
        (status = 404, description = "Cluster not found, disabled, or discovery not enabled."),
        (status = 503, description = "Cache unreachable."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
    )
)]
#[get("/{ns}/{cluster}/.well-known/oauth-authorization-server")]
#[instrument(name = "oauth_authorization_server", skip(data))]
pub async fn oauth_authorization_server(
    req: HttpRequest,
    data: web::Data<State>,
) -> impl Responder {
    let (ns, cluster) = if let Some(parts) = extract_ns_cluster(&req) {
        parts
    } else {
        error!(path = %req.path(), "missing ns/cluster path parameters");
        return HttpResponse::NotFound().finish();
    };
    let proxy = match load_discovery_enabled_proxy(&data, &ns, &cluster).await {
        Ok(proxy) => proxy,
        Err(response) => return response,
    };
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }

    let scopes_supported = proxy
        .spec
        .auth_config
        .as_ref()
        .map(|auth_config| {
            auth_config
                .oidc_provider
                .extra_scope
                .split(' ')
                .filter(|scope| !scope.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let issuer = proxy.to_full_path(data.into_inner());
    let metadata = OAuthAuthorizationServerMetadata {
        authorization_endpoint: format!("{issuer}/oauth/authorize"),
        token_endpoint: format!("{issuer}/oauth/token"),
        jwks_uri: format!("{issuer}/oauth/jwks"),
        issuer,
        scopes_supported,
        response_types_supported: vec!["code".to_string()],
        grant_types_supported: vec!["authorization_code".to_string()],
        token_endpoint_auth_methods_supported: vec!["none".to_string()],
        code_challenge_methods_supported: vec!["S256".to_string()],
    };
    HttpResponse::Ok().json(metadata)
}
