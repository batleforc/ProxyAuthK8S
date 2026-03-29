use actix_web::web;
use common::State;
use crd::ProxyKubeApi;
use kube::ResourceExt;
use rustls::{
    pki_types::{pem::PemObject as _, CertificateDer},
    ClientConfig, RootCertStore,
};
use rustls_platform_verifier::BuilderVerifierExt;

pub(super) async fn build_tls_config(
    proxy: &ProxyKubeApi,
    state: &web::Data<State>,
) -> Result<ClientConfig, String> {
    let cert_pem = proxy
        .spec
        .cert
        .get_cert(state.client.clone(), &proxy.namespace().unwrap_or_default())
        .await?;

    if let Some(cert_pem) = cert_pem {
        let mut root_store = RootCertStore::empty();
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
        root_store.add_parsable_certificates(certs);

        Ok(ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth())
    } else {
        ClientConfig::builder()
            .with_platform_verifier()
            .map_err(|e| e.to_string())
            .map(|builder| builder.with_no_client_auth())
    }
}
