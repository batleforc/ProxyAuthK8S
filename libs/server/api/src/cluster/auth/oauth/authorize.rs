use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd_runtime::ProxyKubeApiRuntime;
use openidconnect::{core::CoreAuthenticationFlow, CsrfToken, Nonce, PkceCodeChallenge, Scope};
use serde::Deserialize;
use tracing::{error, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::{
    load_discovery_enabled_proxy,
    oauth::{
        model::{
            is_valid_pkce_value, parse_loopback_redirect_uri, PendingAuthorization, PENDING_PREFIX,
            PENDING_TTL_SECONDS,
        },
        redirect_with_error,
    },
};
use crate::helper::extract_ns_cluster;

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct AuthorizeQuery {
    /// Must be `code`; this server only implements the authorization code grant.
    pub response_type: String,
    /// Unvalidated: the proxy mediates the whole flow, so external clients
    /// never need to register with the upstream provider.
    pub client_id: String,
    /// Must be a loopback URI (`http://localhost` or `http://127.0.0.1`), any port/path.
    pub redirect_uri: String,
    /// Opaque value echoed back verbatim on redirect.
    pub state: Option<String>,
    pub scope: Option<String>,
    /// RFC 7636 PKCE code challenge (43-128 chars, unreserved charset).
    pub code_challenge: String,
    /// Must be `S256` when present; only S256 is supported.
    pub code_challenge_method: Option<String>,
}

/// Start the mediated OAuth Authorization Server flow for a cluster
///
/// Redirects the caller's browser to the cluster's upstream OIDC provider. If
/// the cluster is not found, disabled, or discovery is not enabled, return 404.
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 302, description = "Redirect to the upstream identity provider."),
        (status = 400, description = "Malformed authorization request (invalid or non-loopback redirect_uri)."),
        (status = 404, description = "Cluster not found, disabled, or discovery not enabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
        AuthorizeQuery,
    )
)]
#[get("/{ns}/{cluster}/oauth/authorize")]
#[instrument(name = "oauth_as_authorize", skip(data, query))]
pub async fn authorize(
    req: HttpRequest,
    data: web::Data<State>,
    query: web::Query<AuthorizeQuery>,
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

    let Some(redirect_uri) = parse_loopback_redirect_uri(&query.redirect_uri) else {
        return HttpResponse::BadRequest()
            .body("invalid_request: redirect_uri must be a loopback URI (http://localhost or http://127.0.0.1)");
    };

    if query.response_type != "code" {
        return redirect_with_error(&redirect_uri, "unsupported_response_type", query.state.as_deref());
    }
    if !is_valid_pkce_value(&query.code_challenge) {
        return redirect_with_error(&redirect_uri, "invalid_request", query.state.as_deref());
    }
    if query
        .code_challenge_method
        .as_deref()
        .is_some_and(|method| method != "S256")
    {
        return redirect_with_error(&redirect_uri, "invalid_request", query.state.as_deref());
    }

    let oauth_conf = if let Some(conf) = proxy.get_oauth_as_oidc_conf(data.clone().into_inner()) {
        conf
    } else {
        error!("OIDC config not found");
        return HttpResponse::InternalServerError().finish();
    };
    let client = match oauth_conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get oidc client");
            return redirect_with_error(&redirect_uri, "server_error", query.state.as_deref());
        }
    };

    let correlation_id = CsrfToken::new_random().secret().clone();
    let nonce_value = Nonce::new_random().secret().clone();
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let scopes: Vec<Scope> = oauth_conf
        .scopes
        .split(' ')
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
    let pending_json = match serde_json::to_string(&pending) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "couldn't serialize pending authorization");
            return HttpResponse::InternalServerError().finish();
        }
    };
    if let Err(e) = data
        .redis_set(
            &format!("{PENDING_PREFIX}:{ns}/{cluster}/{correlation_id}"),
            &pending_json,
            Some(PENDING_TTL_SECONDS),
        )
        .await
    {
        error!(error = %e, "couldn't store pending authorization");
        return HttpResponse::ServiceUnavailable().finish();
    }

    HttpResponse::Found()
        .insert_header(("Location", auth_url.to_string()))
        .finish()
}
