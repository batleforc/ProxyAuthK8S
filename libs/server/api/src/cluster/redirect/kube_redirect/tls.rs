use actix_web::web;
use common::State;
use crd::ProxyKubeApi;
use kube::ResourceExt;
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _},
};
use rustls_platform_verifier::BuilderVerifierExt;

/// TLS configuration used to reach the target cluster.
///
/// The trust anchor comes from the cluster's `cert` (or the platform store when
/// it has none). A client certificate is attached when the cluster is
/// configured for mutual TLS and `attach_client_cert` is `true`; pass `false`
/// to build a CA-only connection (e.g. a privileged bearer-token call, which
/// must authenticate unambiguously as the token and never be conflated with
/// the front-proxy mTLS identity used for impersonated calls).
pub(super) async fn build_tls_config(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
    attach_client_cert: bool,
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

    let client_cert = if attach_client_cert {
        proxy.spec.client_cert.as_ref()
    } else {
        None
    };
    let Some(client_cert) = client_cert else {
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
