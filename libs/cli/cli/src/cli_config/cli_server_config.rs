use crate::keystore;
use crate::{cli_config::cli_cluster_config::CliClusterConfig, error::ProxyAuthK8sError};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use client_api::{
    apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration},
    models::GetAllVisibleClusterBody,
};
use keyring_core::Entry;
use reqwest::Certificate;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};
use tracing::{debug, error};

/// Keyring service every server and cluster token is stored under.
const KEYRING_SERVICE: &str = "proxyauthk8s";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CliServerConfig {
    pub url: String,
    pub namespace: String,
    pub clusters: HashMap<String, CliClusterConfig>,
    /// Base64 PEM bundle of the CA that signed the server's TLS certificate,
    /// for servers whose certificate the system does not trust (self-signed,
    /// internal CA). Same encoding as a kubeconfig `certificate-authority-data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_authority_data: Option<String>,
}

impl CliServerConfig {
    #[must_use]
    pub fn new(server_url: String) -> Self {
        CliServerConfig {
            url: server_url,
            namespace: "default".to_string(),
            clusters: vec![].into_iter().collect(),
            certificate_authority_data: None,
        }
    }
    #[must_use]
    pub fn url_to_name(&self) -> String {
        let url = self.url.replace("https://", "").replace("http://", "");
        url.replace(['.', ':'], "-")
    }

    #[must_use]
    pub fn url_to_name_from_string(url: String) -> String {
        let url = url.replace("https://", "").replace("http://", "");
        url.replace(['.', ':'], "-")
    }

    #[must_use]
    pub fn get_cluster_url_from_ns_name(&self, ns: Option<String>, name: String) -> Option<String> {
        let ns = ns.unwrap_or_else(|| self.namespace.clone());
        format!("{}/{}/{}", self.url, ns, name).into()
    }

    #[must_use]
    pub fn get_clusters_from_ns_name(
        &self,
        ns: Option<String>,
        name: String,
    ) -> Option<&CliClusterConfig> {
        self.clusters.get(&format!(
            "{}/{}",
            ns.unwrap_or_else(|| self.namespace.clone()),
            name
        ))
    }

    pub fn server_token(&self) -> Result<String, ProxyAuthK8sError> {
        self.read_token(&self.url)
    }

    pub fn set_server_token(&self, token: String) -> Result<(), ProxyAuthK8sError> {
        self.write_token(&self.url, &token)
    }

    pub fn clear_server_token(&self) -> Result<(), ProxyAuthK8sError> {
        self.delete_token(&self.url)
    }

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

    pub fn set_cluster_token(
        &mut self,
        ns: String,
        cluster: String,
        token: String,
    ) -> Result<(), ProxyAuthK8sError> {
        let key = format!("{ns}/{cluster}");
        let _ = self
            .clusters
            .entry(key.clone())
            .or_insert(CliClusterConfig { token_exist: true });
        self.write_token(&self.cluster_keyring_user(&key), &token)
    }

    pub fn get_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<String, ProxyAuthK8sError> {
        self.read_token(&self.cluster_keyring_user(&format!("{ns}/{cluster}")))
    }

    pub fn clear_cluster_token(
        &self,
        ns: String,
        cluster: String,
    ) -> Result<(), ProxyAuthK8sError> {
        self.delete_token(&self.cluster_keyring_user(&format!("{ns}/{cluster}")))
    }

    /// Keyring user under which the token of cluster `key` (`"<ns>/<cluster>"`)
    /// is stored. Changing this format would orphan every stored cluster token.
    fn cluster_keyring_user(&self, key: &str) -> String {
        format!("{}::{}", self.url, key)
    }

    /// Open the keyring entry for `user` under [`KEYRING_SERVICE`], mapping a
    /// failure to the caller's error variant (`err`).
    fn keyring_entry(
        &self,
        user: &str,
        err: fn(String) -> ProxyAuthK8sError,
    ) -> Result<Entry, ProxyAuthK8sError> {
        keystore::entry(KEYRING_SERVICE, user).map_err(|e| {
            debug!("Keyring entry creation error: {}", e);
            err(format!(
                "Failed to create keyring entry for server URL: {}",
                self.url
            ))
        })
    }

    fn read_token(&self, user: &str) -> Result<String, ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringReadError)?;
        entry.get_password().map_err(|e| {
            debug!("Keyring read error: {}", e);
            ProxyAuthK8sError::KeyringReadError(format!(
                "Failed to read token from keyring for server URL: {}",
                self.url
            ))
        })
    }

    fn write_token(&self, user: &str, token: &str) -> Result<(), ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringWriteError)?;
        entry.set_password(token).map_err(|e| {
            debug!("Keyring write error: {}", e);
            ProxyAuthK8sError::KeyringWriteError(format!(
                "Failed to write token to keyring for server URL: {}",
                self.url
            ))
        })
    }

    fn delete_token(&self, user: &str) -> Result<(), ProxyAuthK8sError> {
        let entry = self.keyring_entry(user, ProxyAuthK8sError::KeyringDeleteError)?;
        entry.delete_credential().map_err(|e| {
            debug!("Keyring delete error: {}", e);
            ProxyAuthK8sError::KeyringDeleteError(format!(
                "Failed to delete token from keyring for server URL: {}",
                self.url
            ))
        })
    }

    pub fn clear_all_tokens(&self) {
        for cluster in self.clusters.keys() {
            let val: Vec<&str> = cluster.split('/').collect();
            let ns = val.first().unwrap_or(&"").to_string();
            let cluster = val.get(1).unwrap_or(&"").to_string();
            if let Err(err) = self.clear_cluster_token(ns, cluster.clone()) {
                error!("Error clearing token for cluster {}: {}", cluster, err);
            }
        }
        if let Err(err) = self.clear_server_token() {
            error!("Error clearing server token: {}", err);
        }
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

    const TEST_CA: &str = include_str!("../../testdata/test-ca.pem");

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

    #[test]
    fn certificate_authority_data_is_optional_in_the_config_file() {
        let config: CliServerConfig =
            serde_yaml_ng::from_str("url: https://a.b\nnamespace: default\nclusters: {}\n")
                .unwrap();
        assert_eq!(config.certificate_authority_data, None);
        let yaml = serde_yaml_ng::to_string(&config).unwrap();
        assert!(!yaml.contains("certificate_authority_data"));
    }

    #[test]
    fn cluster_keyring_user_keeps_the_stored_token_naming() {
        // Tokens already in users' keyrings are looked up under this exact
        // service/user pair; a format change would make them unreadable.
        assert_eq!(KEYRING_SERVICE, "proxyauthk8s");
        let config = CliServerConfig::new("https://localhost:5437".to_string());
        assert_eq!(
            config.cluster_keyring_user("team-a/prod"),
            "https://localhost:5437::team-a/prod"
        );
    }

    #[test]
    fn url_to_name_strips_scheme_and_encodes_separators() {
        assert_eq!(
            CliServerConfig::url_to_name_from_string("https://localhost:5437".to_string()),
            "localhost-5437"
        );
        assert_eq!(
            CliServerConfig::url_to_name_from_string("http://proxy.example.com".to_string()),
            "proxy-example-com"
        );
        // Instance method agrees with the associated one.
        let config = CliServerConfig::new("https://a.b:1".to_string());
        assert_eq!(config.url_to_name(), "a-b-1");
    }

    #[test]
    fn cluster_url_uses_the_given_namespace_then_falls_back_to_the_default() {
        let config = CliServerConfig::new("https://localhost:5437".to_string());
        // Explicit namespace wins.
        assert_eq!(
            config.get_cluster_url_from_ns_name(Some("team-a".to_string()), "prod".to_string()),
            Some("https://localhost:5437/team-a/prod".to_string())
        );
        // None falls back to the server's default namespace ("default").
        assert_eq!(
            config.get_cluster_url_from_ns_name(None, "prod".to_string()),
            Some("https://localhost:5437/default/prod".to_string())
        );
    }

    #[test]
    fn clusters_are_looked_up_by_ns_and_name() {
        let mut config = CliServerConfig::new("https://localhost:5437".to_string());
        config.clusters.insert(
            "team-a/prod".to_string(),
            CliClusterConfig { token_exist: true },
        );

        assert!(
            config
                .get_clusters_from_ns_name(Some("team-a".to_string()), "prod".to_string())
                .is_some()
        );
        // Wrong namespace -> not found.
        assert!(
            config
                .get_clusters_from_ns_name(Some("team-b".to_string()), "prod".to_string())
                .is_none()
        );
        // None uses the default namespace, which has no "prod" entry here.
        assert!(
            config
                .get_clusters_from_ns_name(None, "prod".to_string())
                .is_none()
        );
    }
}
