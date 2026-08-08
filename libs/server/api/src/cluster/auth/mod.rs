pub mod auth_model;
pub mod callback;
pub mod callback_model;
pub mod login;
pub mod oauth;
pub mod well_known;

use actix_web::HttpResponse;
use common::State;
use crd::ProxyKubeApi;

/// Load a `ProxyKubeApi` and gate it the way every well-known/OAuth-AS
/// endpoint must: unknown cluster, disabled proxy, disabled OIDC provider, or
/// discovery not opted into all read as a plain 404 — these endpoints are
/// unauthenticated, so they must not distinguish "doesn't exist" from
/// "exists but isn't opted in" for an anonymous caller.
pub(crate) async fn load_discovery_enabled_proxy(
    data: &State,
    ns: &str,
    cluster: &str,
) -> Result<ProxyKubeApi, HttpResponse> {
    let proxy: ProxyKubeApi = match data
        .get_object_from_redis(crd::REDIS_PREFIX, &format!("{ns}/{cluster}"))
        .await
    {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return Err(HttpResponse::NotFound().finish()),
        Err(e) => {
            tracing::error!(error = %e, "couldn't get proxy from redis");
            return Err(HttpResponse::ServiceUnavailable().finish());
        }
    };
    let discovery_exposed = proxy.spec.auth_config.as_ref().is_some_and(|auth_config| {
        auth_config.oidc_provider.enabled && auth_config.oidc_provider.expose_oauth_authorization_server
    });
    if !proxy.spec.enabled || !discovery_exposed {
        return Err(HttpResponse::NotFound().finish());
    }
    Ok(proxy)
}
