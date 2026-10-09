//! Calls to the `ProxyAuthK8S` server through the generated `client_api`.

use client_api::{
    apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration},
    models::GetAllVisibleClusterBody,
};
use std::path::Path;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use reqwest::Certificate;
use tracing::debug;

use crate::{cli_config::cli_server_config::CliServerConfig, error::ProxyAuthK8sError};

impl CliServerConfig {
    pub fn base_configuration(&self) -> Result<Configuration, ProxyAuthK8sError> {
        let token = self.server_token()?;
        Ok(Configuration {
            base_path: self.url.clone(),
            bearer_access_token: Some(token),
            client: http_client(self.certificate_authority_data.as_deref())?,
            ..Default::default()
        })
    }

    pub async fn clusters_from_remote(
        &self,
    ) -> Result<GetAllVisibleClusterBody, ProxyAuthK8sError> {
        get_all_visible_cluster(&self.base_configuration()?)
            .await
            .map_err(|e| {
                debug!("Error fetching clusters from remote: {:?}", e);
                e.into()
            })
    }
}

/// Read a PEM CA bundle from `path` and return it base64-encoded, after
/// checking it holds at least one certificate.
pub fn load_certificate_authority(path: &Path) -> Result<String, ProxyAuthK8sError> {
    let pem = std::fs::read(path).map_err(|e| {
        ProxyAuthK8sError::InvalidCertificateAuthority(format!(
            "failed to read {}: {e}",
            path.to_string_lossy()
        ))
    })?;
    parse_certificates(&pem)?;
    Ok(BASE64.encode(pem))
}

fn parse_certificates(pem: &[u8]) -> Result<Vec<Certificate>, ProxyAuthK8sError> {
    let certs = Certificate::from_pem_bundle(pem)
        .map_err(|e| ProxyAuthK8sError::InvalidCertificateAuthority(e.to_string()))?;
    if certs.is_empty() {
        return Err(ProxyAuthK8sError::InvalidCertificateAuthority(
            "no PEM certificate found".to_string(),
        ));
    }
    Ok(certs)
}

/// HTTP client for the `ProxyAuthK8S` API, trusting `certificate_authority_data`
/// (base64 PEM) on top of the system roots when it is set.
pub fn http_client(
    certificate_authority_data: Option<&str>,
) -> Result<reqwest::Client, ProxyAuthK8sError> {
    let mut builder = reqwest::Client::builder();
    if let Some(data) = certificate_authority_data {
        let pem = BASE64.decode(data).map_err(|e| {
            ProxyAuthK8sError::InvalidCertificateAuthority(format!(
                "stored certificate_authority_data is not valid base64: {e}"
            ))
        })?;
        builder = builder.tls_certs_merge(parse_certificates(&pem)?);
    }
    builder
        .build()
        .map_err(|e| ProxyAuthK8sError::InvalidCertificateAuthority(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CA: &str = include_str!("../../../testdata/test-ca.pem");

    fn scratch_file(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("proxyauth-cli-ca-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_certificate_authority_returns_the_base64_pem() {
        let path = scratch_file("ca.pem", TEST_CA);
        let data = load_certificate_authority(&path).expect("a PEM certificate loads");
        assert_eq!(BASE64.decode(&data).unwrap(), TEST_CA.as_bytes());
        // The stored form is what the HTTP client consumes.
        http_client(Some(&data)).expect("the stored CA builds a client");
    }

    #[test]
    fn load_certificate_authority_rejects_a_file_without_certificate() {
        let path = scratch_file("not-a-ca.pem", "hello");
        assert!(matches!(
            load_certificate_authority(&path),
            Err(ProxyAuthK8sError::InvalidCertificateAuthority(_))
        ));
        assert!(matches!(
            load_certificate_authority(Path::new("/nonexistent/ca.pem")),
            Err(ProxyAuthK8sError::InvalidCertificateAuthority(_))
        ));
    }

    #[test]
    fn http_client_rejects_corrupted_stored_data() {
        assert!(matches!(
            http_client(Some("not base64!")),
            Err(ProxyAuthK8sError::InvalidCertificateAuthority(_))
        ));
        http_client(None).expect("no CA means the system roots");
    }
}
