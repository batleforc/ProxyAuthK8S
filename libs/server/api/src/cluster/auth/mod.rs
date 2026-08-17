pub mod auth_model;
pub mod callback;
pub mod callback_model;
pub mod login;
pub mod oauth;
pub mod well_known;

use actix_web::{HttpRequest, HttpResponse};
use common::State;
use crd::ProxyKubeApi;

use crate::cluster::redirect::{forwarded, status_response::too_many_requests, throttle};

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
        auth_config.oidc_provider.enabled
            && auth_config.oidc_provider.expose_oauth_authorization_server
    });
    if !proxy.spec.enabled || !discovery_exposed {
        return Err(HttpResponse::NotFound().finish());
    }
    Ok(proxy)
}

/// Rate-limit / ban check shared by every well-known and OAuth-AS endpoint.
///
/// These endpoints are unauthenticated by design, so unlike `redirect()` there
/// is never a resolved user to key a budget on — every check is keyed on the
/// caller's address instead, the same way `redirect()` throttles the
/// pre-authentication half of its own request. Reuses the cluster's own
/// `SecurityConfiguration` (`rate_limiting`/`fail2login_equal_ban`), so an
/// admin gets one shared, already-tuned budget across the proxy path and the
/// discovery/OAuth-AS surface rather than a second limit to configure.
pub(crate) async fn throttle_oauth_as(
    req: &HttpRequest,
    data: &State,
    proxy: &ProxyKubeApi,
) -> Option<HttpResponse> {
    let peer_ip = req.peer_addr().map(|addr| addr.ip());
    let forwarded_for = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    let peer_id = forwarded::throttle_client_ip(forwarded_for, peer_ip);

    if throttle::is_banned(data, proxy, &peer_id).await {
        let retry_after = throttle::ban_retry_after(data, proxy, &peer_id).await;
        let throttled = throttle::Throttled::Banned { retry_after };
        tracing::warn!(peer = %peer_id, "refusing a banned client on the OAuth-AS surface");
        return Some(too_many_requests(
            &throttled.message(),
            throttled.retry_after(),
        ));
    }

    if let Some(throttled) = throttle::check_rate_limit(data, proxy, &peer_id, &[]).await {
        return Some(too_many_requests(
            &throttled.message(),
            throttled.retry_after(),
        ));
    }
    None
}
