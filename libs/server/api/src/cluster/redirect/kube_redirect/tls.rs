use std::sync::Arc;

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
    state: &State,
) -> Result<Arc<ClientConfig>, String> {
    UPSTREAM_TLS_CONFIGS
        .get_or_try_insert_with(
            &UpstreamCacheKey::for_proxy(proxy),
            upstream_client_ttl(),
            || async {
                build_tls_config(proxy, state, ClientCert::Configured)
                    .await
                    .map(Arc::new)
            },
        )
        .await
}

/// Which client certificate a TLS configuration presents.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClientCert {
    /// None: a CA-only connection (e.g. a privileged bearer-token call, which
    /// must authenticate as the token alone).
    None,
    /// The one the cluster's mode calls for: `client_cert`, or in
    /// `FrontProxy` the Secret of the proxy's namespace.
    Configured,
    /// The `FrontProxy` Secret whatever the mode: the trust probe of a
    /// cluster still preparing the switch (RFC 0014 §5.4).
    FrontProxy,
}

/// TLS configuration used to reach the target cluster.
///
/// The trust anchor comes from the cluster's `cert` (or the platform store when
/// it has none). A client certificate is attached when the cluster is
/// configured for mutual TLS, as [`ClientCert`] says.
pub(super) async fn build_tls_config(
    proxy: &ProxyKubeApi,
    state: &State,
    client: ClientCert,
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

    if client == ClientCert::None {
        return Ok(builder.with_no_client_auth());
    }
    // RFC 0014: in FrontProxy the certificate is the one the proxy
    // administrator put in the proxy's namespace for this cluster and this
    // target, never a `client_cert` (refused by admission in this mode).
    let (cert_pem, key_pem) = if client == ClientCert::FrontProxy
        || proxy.upstream_mode() == crd::upstream_auth::UpstreamAuthMode::FrontProxy
    {
        crd::upstream_auth::front_proxy::load(state.client.clone(), &state.lease_namespace, proxy)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let Some(client_cert) = proxy.spec.client_cert.as_ref() else {
            return Ok(builder.with_no_client_auth());
        };
        client_cert
            .resolve(state.client.clone(), &namespace)
            .await
            .map_err(|e| e.to_string())?
    };
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
