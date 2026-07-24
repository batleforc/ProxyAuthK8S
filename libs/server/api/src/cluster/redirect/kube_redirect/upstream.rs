//! Building the upstream request, shared by the standard and virtual paths.

use actix_web::{dev::PeerAddr, web, HttpRequest};
use common::State;
use crd::ProxyKubeApi;
use tracing::info;

use super::tls::build_tls_config;
use crate::cluster::redirect::forwarded::{
    forwarded_for_value, identity_headers, is_hop_by_hop, is_proxy_owned_header,
    is_upstream_auth_header,
};
use crate::model::user::User;

/// A reqwest client trusting whatever the cluster's `CertSource` says.
pub(super) async fn upstream_client(
    proxy: &ProxyKubeApi,
    data: &web::Data<State>,
) -> Result<reqwest::Client, String> {
    let tls_config = build_tls_config(proxy, data).await?;
    reqwest::ClientBuilder::new()
        .use_preconfigured_tls(tls_config)
        // Never follow redirects to the upstream: a Kubernetes apiserver does not
        // 30x proxied API calls, and following one would let a configured target
        // bounce the request (and the forwarded bearer token) to an unintended
        // host — e.g. the cloud metadata endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| err.to_string())
}

/// Copy the client headers onto `builder`, then stamp the proxy-owned ones.
///
/// Headers the proxy is authoritative for are dropped from the client request
/// and re-created here, so a caller cannot forge its own identity.
pub(super) fn apply_forward_headers(
    mut builder: reqwest::RequestBuilder,
    req: &HttpRequest,
    peer_addr: Option<PeerAddr>,
    user: Option<&User>,
) -> reqwest::RequestBuilder {
    for (name, value) in req.headers().iter() {
        let name = name.as_str();
        // Skip headers that must not be forwarded or are managed by reqwest when
        // streaming, any header the proxy itself is authoritative for, and any
        // client-supplied upstream-auth/impersonation header.
        if is_hop_by_hop(name) || is_proxy_owned_header(name) || is_upstream_auth_header(name) {
            continue;
        }

        // Only forward header values that are valid UTF-8 strings. If not valid, skip them.
        match value.to_str() {
            Ok(value) => builder = builder.header(name, value),
            Err(_) => info!(header = %name, "skipping non-utf8 header"),
        }
    }

    // Append to the forwarding chain rather than replacing it, so an upstream
    // proxy's record of the original client survives.
    let incoming_forwarded_for = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    let peer_ip = peer_addr.map(|PeerAddr(addr)| addr.ip());
    if let Some(forwarded_for) = forwarded_for_value(incoming_forwarded_for, peer_ip) {
        builder = builder.header("x-forwarded-for", forwarded_for);
    }

    for (name, value) in identity_headers(user) {
        builder = builder.header(name, value);
    }

    builder
}
