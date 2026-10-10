//! Fixtures shared by the `virtual_api*` integration binaries.
//!
//! The suite is split by subject across several binaries (mapping,
//! list-fallback filtering, rules-review resolution, allow-list interaction);
//! the proxy fixtures they all seed live here so the split does not duplicate
//! them. Mirrors `envtest_support` for the envtest tier.
//!
//! Not every binary uses every fixture, hence the `dead_code` allowance.

#![allow(dead_code)]

use base64::{Engine, prelude::BASE64_STANDARD};
use crd::certificate::CertSource;
use crd::virtual_api::VirtualApiKind;

use crate::harness::{proxy_fixture, seed_proxy, with_virtual_api};

/// A proxy fixture with the OpenShift Project virtual API turned on.
pub async fn seed_openshift_proxy(
    pool: &deadpool_redis::Pool,
    ns: &str,
    cluster: &str,
    upstream_url: &str,
) {
    let mut proxy = proxy_fixture(ns, cluster, upstream_url);
    with_virtual_api(&mut proxy, VirtualApiKind::OpenShiftProject);
    seed_proxy(pool, &proxy).await;
}

/// The bearer token used by the `list_fallback_token`-enabled fixtures below.
pub const FALLBACK_TOKEN: &str = "test-fallback-token";

/// A proxy fixture with the OpenShift Project virtual API turned on and a
/// `list_fallback_token` configured, so `LIST projects` is unconditionally
/// filtered per namespace instead of a plain forwarded `LIST namespaces`.
pub async fn seed_openshift_proxy_with_fallback(
    pool: &deadpool_redis::Pool,
    ns: &str,
    cluster: &str,
    upstream_url: &str,
) {
    let mut proxy = proxy_fixture(ns, cluster, upstream_url);
    with_virtual_api(&mut proxy, VirtualApiKind::OpenShiftProject);
    proxy.spec.virtual_apis[0].list_fallback_token =
        Some(CertSource::Cert(BASE64_STANDARD.encode(FALLBACK_TOKEN)));
    seed_proxy(pool, &proxy).await;
}
