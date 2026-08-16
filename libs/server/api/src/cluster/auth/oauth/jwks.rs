use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use common::State;
use crd_runtime::ProxyKubeApiRuntime;
use serde_json::Value;
use tracing::{error, instrument};

use crate::cluster::auth::{load_discovery_enabled_proxy, throttle_oauth_as};
use crate::helper::extract_ns_cluster;

/// The upstream identity provider's JSON Web Key Set, mirrored under the cluster's own path
///
/// Stateless passthrough of the upstream provider's `jwks_uri`, so a caller
/// verifying a token never needs to learn or contact the upstream provider's
/// own hostname. If the cluster is not found, disabled, or does not have
/// discovery enabled, return 404.
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 200, description = "The upstream identity provider's JSON Web Key Set."),
        (status = 404, description = "Cluster not found, disabled, or discovery not enabled."),
        (status = 503, description = "Upstream OIDC provider unreachable or its discovery document is invalid."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
    )
)]
#[get("/{ns}/{cluster}/oauth/jwks")]
#[instrument(name = "oauth_as_jwks", skip(data))]
pub async fn jwks(req: HttpRequest, data: web::Data<State>) -> impl Responder {
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

    let oidc_conf = if let Some(conf) = proxy.get_oidc_conf(data.into_inner(), false, None) {
        conf
    } else {
        error!("OIDC config not found");
        return HttpResponse::InternalServerError().finish();
    };
    let client = match oidc_conf.reqwest_client() {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't build oidc http client");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        oidc_conf.issuer_url.trim_end_matches('/')
    );
    let upstream: Value = match client.get(&discovery_url).send().await {
        Ok(response) => match response.json().await {
            Ok(json) => json,
            Err(e) => {
                error!(error = %e, "couldn't parse upstream discovery document");
                return HttpResponse::ServiceUnavailable().finish();
            }
        },
        Err(e) => {
            error!(error = %e, "couldn't reach upstream OIDC provider");
            return HttpResponse::ServiceUnavailable().finish();
        }
    };
    let Some(jwks_uri) = upstream.get("jwks_uri").and_then(Value::as_str) else {
        error!("upstream discovery document is missing jwks_uri");
        return HttpResponse::ServiceUnavailable().finish();
    };

    match client.get(jwks_uri).send().await {
        Ok(response) => match response.json::<Value>().await {
            Ok(jwks) => HttpResponse::Ok().json(jwks),
            Err(e) => {
                error!(error = %e, "couldn't parse upstream jwks document");
                HttpResponse::ServiceUnavailable().finish()
            }
        },
        Err(e) => {
            error!(error = %e, "couldn't reach upstream jwks endpoint");
            HttpResponse::ServiceUnavailable().finish()
        }
    }
}
