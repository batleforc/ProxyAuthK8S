use std::sync::Arc;

use actix_web::web;
use common::{
    State,
    upstream_cache::{UPSTREAM_TLS_CONFIGS, UpstreamCacheKey, upstream_client_ttl},
};
use crd::ProxyKubeApi;
use kube::ResourceExt;
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _},
};
use rustls_platform_verifier::BuilderVerifierExt;

/// [`build_tls_config`], cached per cluster (see [`common::upstream_cache`]):
/// the upgrade path opens a raw connection per stream and would otherwise
/// re-read the cluster's certificates from Kubernetes every time.
pub(super) async fn cached_tls_config(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
) -> Result<Arc<ClientConfig>, String> {
    UPSTREAM_TLS_CONFIGS
        .get_or_try_insert_with(
            &UpstreamCacheKey::for_proxy(proxy),
            upstream_client_ttl(),
            || async { build_tls_config(proxy, state).await.map(Arc::new) },
        )
        .await
}

/// TLS configuration used to reach the target cluster.
///
/// The trust anchor comes from the cluster's `cert` (or the platform store when
/// it has none), and a client certificate is attached when the cluster is
/// configured for mutual TLS.
pub(super) async fn build_tls_config(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
) -> Result<ClientConfig, String> {
    let namespace = proxy.namespace().unwrap_or_default();
    let cert_pem = proxy
        .spec
        .cert
        .get_cert(state.client.clone(), &namespace)
        .await
        .map_err(|e| e.to_string())?;

    let builder = ClientConfig::builder();
    let builder = if let Some(cert_pem) = cert_pem {
        let mut root_store = RootCertStore::empty();
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
        root_store.add_parsable_certificates(certs);

        builder.with_root_certificates(root_store)
    } else {
        builder
            .with_platform_verifier()
            .map_err(|e| e.to_string())?
    };

    let Some(client_cert) = &proxy.spec.client_cert else {
        return Ok(builder.with_no_client_auth());
    };

    let (cert_pem, key_pem) = client_cert
        .resolve(state.client.clone(), &namespace)
        .await
        .map_err(|e| e.to_string())?;
    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("invalid mTLS client certificate: {e}"))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|e| format!("invalid mTLS client key: {e}"))?;

    builder
        .with_client_auth_cert(cert_chain, key)
        .map_err(|e| format!("could not configure mTLS: {e}"))
}
