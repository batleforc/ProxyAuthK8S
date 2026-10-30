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

#[cfg(test)]
mod tests {
    use super::*;
    use crd::ProxyKubeApiSpec;
    use crd::certificate::CertSource;
    use crd::service::Service;
    use crd::upstream_auth::{UpstreamAuth, UpstreamAuthMode};

    fn front_proxy_cluster() -> ProxyKubeApi {
        let mut proxy = ProxyKubeApi::new(
            "fp",
            ProxyKubeApiSpec {
                enabled: true,
                cert: CertSource::SystemRoots(true),
                client_cert: None,
                service: Service::ExternalService {
                    url: "https://cluster.example.com".to_string(),
                },
                auth_config: None,
                security_config: None,
                expose_via_dashboard: false,
                dashboard_group: None,
                proxy_group: None,
                virtual_apis: Vec::new(),
                upstream_auth: Some(UpstreamAuth {
                    mode: UpstreamAuthMode::FrontProxy,
                    user_prefix: Some("proxyauth:".to_string()),
                    group_prefix: Some("proxyauth:".to_string()),
                    issuer_key: None,
                }),
            },
        );
        proxy.metadata.namespace = Some("default".to_string());
        proxy
    }

    /// A kube client on a closed port: reading the front-proxy Secret fails.
    fn state() -> State {
        drop(rustls::crypto::ring::default_provider().install_default());
        let config = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
        State::from_parts(
            kube::Client::try_from(config).unwrap(),
            common::kv_memory::InMemoryKv::new(),
            common::oidc_conf::OidcConf {
                client_id: "proxyauthk8s".to_string(),
                client_secret: None,
                issuer_url: "https://idp.example.com".to_string(),
                scopes: "openid".to_string(),
                audience: "proxyauthk8s".to_string(),
                accept_authorized_party: false,
                redirect_url: None,
            },
            "https://proxy.example.com".to_string(),
            "https://front.example.com".to_string(),
        )
    }

    /// RFC 0014 T17: the `list_fallback` client of a `FrontProxy` cluster
    /// presents no client certificate, so its privileged token is never
    /// paired with the asserted front-proxy identity; the Secret holding that
    /// certificate is not even read (here it cannot be).
    #[tokio::test]
    async fn the_ca_only_client_of_a_front_proxy_cluster_presents_no_certificate() {
        let (proxy, state) = (front_proxy_cluster(), state());
        let config = build_tls_config(&proxy, &state, ClientCert::None)
            .await
            .expect("a CA-only config needs no Secret");
        assert!(!config.client_auth_cert_resolver.has_certs());

        // The mode's own client does go for the front-proxy Secret.
        assert!(
            build_tls_config(&proxy, &state, ClientCert::Configured)
                .await
                .is_err()
        );
    }
}
