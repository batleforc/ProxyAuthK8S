//! Building the upstream request, shared by the standard and virtual paths.

use std::{sync::Arc, time::Duration};

use axum::http::HeaderMap;
use common::{
    State,
    upstream_cache::{
        UPSTREAM_CLIENTS, UPSTREAM_TLS_CONFIGS, UpstreamCacheKey, upstream_client_ttl,
    },
};
use crd::ProxyKubeApi;
use crd::service::Service;
use kube::ResourceExt;
use tracing::info;

use super::tls::build_tls_config;
use crate::cluster::redirect::forwarded::{
    forwarded_for_value, identity_headers, is_hop_by_hop, is_proxy_owned_header,
    is_upstream_auth_header,
};
use crate::model::user::User;

/// Upper bound on establishing the TCP + TLS connection to the target apiserver.
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A reqwest client trusting whatever the cluster's `CertSource` says.
///
/// Cached per cluster (see [`common::upstream_cache`]) so its connection pool
/// keeps connections to the apiserver alive across requests. A new client
/// always resolves the TLS material afresh (and refreshes the cached TLS
/// configuration the upgrade path uses), so neither outlives the TTL.
pub(super) async fn upstream_client(
    proxy: &ProxyKubeApi,
    data: &State,
) -> Result<reqwest::Client, String> {
    let key = UpstreamCacheKey::for_proxy(proxy);
    let ttl = upstream_client_ttl();
    UPSTREAM_CLIENTS
        .get_or_try_insert_with(&key, ttl, || async {
            let tls_config = Arc::new(build_tls_config(proxy, data, true).await?);
            UPSTREAM_TLS_CONFIGS.insert(key.clone(), Arc::clone(&tls_config), ttl);
            build_upstream_client((*tls_config).clone(), agent_proxy(proxy, data).await?)
        })
        .await
}

/// A reqwest client trusting the cluster's CA but never presenting a client
/// certificate, even when the cluster is configured for mutual TLS.
///
/// For a privileged bearer-token call (see `list_fallback`): its identity must
/// never be conflated with the front-proxy mTLS identity used for impersonated
/// calls, so it authenticates by the token alone. Deliberately kept out of the
/// upstream client cache, whose key does not tell the two identities apart.
pub(super) async fn ca_only_client(
    proxy: &ProxyKubeApi,
    data: &State,
) -> Result<reqwest::Client, String> {
    build_upstream_client(
        build_tls_config(proxy, data, false).await?,
        agent_proxy(proxy, data).await?,
    )
}

/// What a `CONNECT` to the agent gateway cannot reach: a proxy URL that
/// fails, so a cluster whose gateway pods are all gone never falls back to
/// a direct connection (reqwest connects directly when the closure answers
/// `None`).
const NO_GATEWAY: &str = "http://agent-gateway-unavailable.invalid:9";

/// For an `Agent` cluster, reqwest's route through the agent gateway
/// (RFC 0012, decision 25): a `CONNECT` to the pod holding the tunnel, chosen
/// again for every new connection; `None` for any other cluster.
async fn agent_proxy(proxy: &ProxyKubeApi, data: &State) -> Result<Option<reqwest::Proxy>, String> {
    if !matches!(proxy.spec.service, Service::Agent(_)) {
        return Ok(None);
    }
    let uid = proxy
        .uid()
        .ok_or_else(|| "agent cluster without a uid".to_string())?;
    let route = data
        .agent_gateway
        .route(data, &uid)
        .await
        .map_err(|err| err.to_string())?;
    let gateway = Arc::clone(&data.agent_gateway);
    let picked = uid.clone();
    Ok(Some(
        reqwest::Proxy::custom(move |_| {
            Some(gateway.cached_route(&picked).map_or_else(
                || NO_GATEWAY.to_string(),
                |route| format!("http://{}", route.address),
            ))
        })
        .headers(route.headers),
    ))
}

/// The upstream client for `tls_config`: no redirects, bounded connect, and
/// for an `Agent` cluster no route but its gateway `CONNECT`.
fn build_upstream_client(
    tls_config: rustls::ClientConfig,
    agent_proxy: Option<reqwest::Proxy>,
) -> Result<reqwest::Client, String> {
    let builder = match agent_proxy {
        Some(agent_proxy) => reqwest::ClientBuilder::new().proxy(agent_proxy),
        None => reqwest::ClientBuilder::new(),
    };
    builder
        .use_preconfigured_tls(tls_config)
        // Never follow redirects to the upstream: a Kubernetes apiserver does not
        // 30x proxied API calls, and following one would let a configured target
        // bounce the request (and the forwarded bearer token) to an unintended
        // host — e.g. the cloud metadata endpoint.
        .redirect(reqwest::redirect::Policy::none())
        // Fail fast on an unreachable/black-holed apiserver instead of pinning a
        // worker. Only the connect phase is bounded: proxied calls include
        // long-lived `watch`/`exec`/`logs -f` streams that a total or read
        // timeout would cut.
        .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
        .build()
        .map_err(|err| err.to_string())
}

/// Copy the client headers onto `builder`, then stamp the proxy-owned ones.
///
/// Headers the proxy is authoritative for are dropped from the client request
/// and re-created here, so a caller cannot forge its own identity.
pub(super) fn apply_forward_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HeaderMap,
    peer_ip: Option<std::net::IpAddr>,
    user: Option<&User>,
) -> reqwest::RequestBuilder {
    for (name, value) in headers {
        let name = name.as_str();
        // Skip headers that must not be forwarded or are managed by reqwest when
        // streaming, any header the proxy itself is authoritative for, and any
        // client-supplied upstream-auth/impersonation header.
        if is_hop_by_hop(name) || is_proxy_owned_header(name) || is_upstream_auth_header(name) {
            continue;
        }

        // Only forward header values that are valid UTF-8 strings. If not valid, skip them.
        if let Ok(value) = value.to_str() {
            builder = builder.header(name, value)
        } else {
            info!(header = %name, "skipping non-utf8 header")
        }
    }

    // Append to the forwarding chain rather than replacing it, so an upstream
    // proxy's record of the original client survives.
    let incoming_forwarded_for = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    if let Some(forwarded_for) = forwarded_for_value(incoming_forwarded_for, peer_ip) {
        builder = builder.header("x-forwarded-for", forwarded_for);
    }

    for (name, value) in identity_headers(user) {
        builder = builder.header(name, value);
    }

    builder
}
