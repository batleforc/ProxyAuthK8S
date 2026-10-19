use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use crd_runtime::ProxyKubeApiRuntime;
use serde_json::Value;
use tracing::{error, instrument};

use crate::cluster::auth::{load_discovery_enabled_proxy, throttle_oauth_as};
use crate::helper::NsCluster;

/// The upstream identity provider's JSON Web Key Set, mirrored under the cluster's own path
///
/// Stateless passthrough of the upstream provider's `jwks_uri`, so a caller
/// verifying a token never needs to learn or contact the upstream provider's
/// own hostname. If the cluster is not found, disabled, or does not have
/// discovery enabled, return 404.
#[utoipa::path(
    get,
    path = "/{ns}/{cluster}/oauth/jwks",
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
#[instrument(name = "oauth_as_jwks", skip(req, data))]
pub async fn jwks(
    req: Parts,
    State(data): State<Arc<common::State>>,
    Path(NsCluster { ns, cluster }): Path<NsCluster>,
) -> Response {
    let proxy = match load_discovery_enabled_proxy(&data, &ns, &cluster).await {
        Ok(proxy) => proxy,
        Err(gate) => return gate.into_response(),
    };
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }

    let oidc_conf = match proxy.get_oidc_conf(data.clone(), false, None).await {
        Ok(Some(conf)) => conf,
        Ok(None) => {
            error!("OIDC config not found");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(e) => {
            // The cluster does have OIDC configured, we just could not read it
            // (typically a `config_from` Secret that is missing or unreadable).
            error!(error = %e, "couldn't resolve the OIDC config");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let client = match oidc_conf.reqwest_client() {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't build oidc http client");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
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
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
        },
        Err(e) => {
            error!(error = %e, "couldn't reach upstream OIDC provider");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let Some(jwks_uri) = upstream.get("jwks_uri").and_then(Value::as_str) else {
        error!("upstream discovery document is missing jwks_uri");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    match client.get(jwks_uri).send().await {
        Ok(response) => match response.json::<Value>().await {
            Ok(jwks) => (StatusCode::OK, Json(jwks)).into_response(),
            Err(e) => {
                error!(error = %e, "couldn't parse upstream jwks document");
                StatusCode::SERVICE_UNAVAILABLE.into_response()
            }
        },
        Err(e) => {
            error!(error = %e, "couldn't reach upstream jwks endpoint");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}
