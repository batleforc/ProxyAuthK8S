//! Calls to the `ProxyAuthK8S` server through the generated `client_api`.

use client_api::{
    apis::{api_clusters_api::get_all_visible_cluster, configuration::Configuration},
    models::GetAllVisibleClusterBody,
};
use std::path::Path;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use reqwest::Certificate;
use tracing::debug;
use url::{Host, Url};

use crate::{cli_config::cli_server_config::CliServerConfig, error::ProxyAuthK8sError};

/// The `client_api` configuration for `url`, built field by field: the
/// generated `Default` starts from `http://localhost` and a generator user
/// agent, neither of which a request of ours should carry.
pub fn api_configuration(
    url: &str,
    bearer_access_token: Option<String>,
    client: reqwest::Client,
) -> Configuration {
    Configuration {
        base_path: url.to_string(),
        user_agent: Some(concat!("kubectl-proxyauth/", env!("CARGO_PKG_VERSION")).to_string()),
        client,
        basic_auth: None,
        oauth_access_token: None,
        bearer_access_token,
        api_key: None,
    }
}

impl CliServerConfig {
    pub fn base_configuration(&self) -> Result<Configuration, ProxyAuthK8sError> {
        // Checked before the keyring is read: a refused URL never sees the token.
        require_secure_server_url(&self.url)?;
        let token = self.server_token()?;
        Ok(api_configuration(
            &self.url,
            Some(token),
            http_client(self.certificate_authority_data.as_deref())?,
        ))
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

/// Refuse to send a credential to `url` in cleartext. Every call to the server
/// carries a bearer token (and the SSO callback an authorization code), so the
/// URL must be `https`; plain `http` is only accepted to a loopback host
/// (`localhost`, `127.0.0.0/8`, `::1`), for local development.
pub fn require_secure_server_url(url: &str) -> Result<(), ProxyAuthK8sError> {
    let refuse = |reason: &str| {
        Err(ProxyAuthK8sError::InvalidServerUrl(
            url.to_string(),
            reason.to_string(),
        ))
    };
    let parsed = match Url::parse(url) {
        Ok(parsed) => parsed,
        Err(e) => return refuse(&e.to_string()),
    };
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => match parsed.host() {
            Some(Host::Domain(domain)) if domain.eq_ignore_ascii_case("localhost") => Ok(()),
            Some(Host::Ipv4(ip)) if ip.is_loopback() => Ok(()),
            Some(Host::Ipv6(ip)) if ip.is_loopback() => Ok(()),
            _ => refuse(
                "plain http would send the token in cleartext; use https \
                 (http is only accepted to localhost / a loopback address)",
            ),
        },
        other => refuse(&format!("unsupported scheme '{other}', use https")),
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
    parse_pem_bundle(&pem)?;
    Ok(BASE64.encode(pem))
}

fn parse_pem_bundle(pem: &[u8]) -> Result<Vec<Certificate>, ProxyAuthK8sError> {
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
        builder = builder.tls_certs_merge(parse_pem_bundle(&pem)?);
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
    fn only_https_or_loopback_http_server_urls_are_accepted() {
        for ok in [
            "https://proxy.example",
            "https://10.0.0.1:8443/base",
            "http://localhost:5437",
            "http://LOCALHOST",
            "http://127.0.0.1:9",
            "http://127.1.2.3",
            "http://[::1]:8080",
        ] {
            require_secure_server_url(ok).unwrap_or_else(|e| panic!("{ok} refused: {e}"));
        }
        for refused in [
            "http://proxy.example",
            "http://10.0.0.1:8080",
            "http://localhost.evil.example",
            "http://127.0.0.1.nip.io",
            "http://[::ffff:7f00:1]",
            "ftp://proxy.example",
            "not a url",
        ] {
            assert!(
                matches!(
                    require_secure_server_url(refused),
                    Err(ProxyAuthK8sError::InvalidServerUrl(_, _))
                ),
                "{refused} accepted"
            );
        }
    }

    #[test]
    fn a_cleartext_remote_url_never_reads_the_token() {
        // No token stored for it either: the URL is refused first, so the
        // error is not the keyring's.
        let config = CliServerConfig::new("http://proxy.example/remote-cleartext".to_string());
        assert!(matches!(
            config.base_configuration(),
            Err(ProxyAuthK8sError::InvalidServerUrl(_, _))
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

    // --- against a mocked API -------------------------------------------

    use crate::test_support::{mount_clusters, mount_clusters_error, tagged_url};
    use wiremock::MockServer;

    #[tokio::test]
    async fn clusters_from_remote_sends_the_stored_token() {
        let server = MockServer::start().await;
        mount_clusters(&server, "remote-tok").await;
        let config = CliServerConfig::new(tagged_url(&server, "remote-ok"));
        config.set_server_token("remote-tok".to_string()).unwrap();

        let body = config.clusters_from_remote().await.unwrap();
        assert_eq!(body.clusters.len(), 2);
        assert_eq!(body.clusters[0].name, "prod");
        assert_eq!(body.clusters[0].is_reachable, Some(Some(true)));
        assert!(body.clusters[1].sso_enabled);
    }

    #[tokio::test]
    async fn base_configuration_needs_a_token_and_a_valid_stored_ca() {
        // No stored token: no request is sent.
        let config = CliServerConfig::new("http://127.0.0.1:9/remote-no-token".to_string());
        assert!(matches!(
            config.clusters_from_remote().await,
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));

        let mut config = CliServerConfig::new("http://127.0.0.1:9/remote-bad-ca".to_string());
        config.set_server_token("tok".to_string()).unwrap();
        config.certificate_authority_data = Some("not base64!".to_string());
        assert!(matches!(
            config.base_configuration(),
            Err(ProxyAuthK8sError::InvalidCertificateAuthority(_))
        ));
    }

    #[tokio::test]
    async fn clusters_from_remote_maps_server_errors() {
        // (status, body, expect Unauthenticated). The status decides, not
        // the body: the server's own 401 has an empty body, and `[]` must not
        // turn a 500 into a re-login prompt.
        let cases = [
            (401, "[]", true),
            (401, "", true),
            (500, "", false),
            (500, "[]", false),
            (503, "{\"reason\":\"down\"}", false),
        ];
        for (status, body, unauthenticated) in cases {
            let server = MockServer::start().await;
            mount_clusters_error(&server, status, body).await;
            let config = CliServerConfig::new(tagged_url(&server, "remote-errors"));
            config.set_server_token("tok".to_string()).unwrap();

            let err = config.clusters_from_remote().await.unwrap_err();
            let matched = if unauthenticated {
                matches!(err, ProxyAuthK8sError::Unauthenticated(_))
            } else {
                matches!(err, ProxyAuthK8sError::RemoteServerError(_))
            };
            assert!(matched, "status {status} body {body:?} gave {err:?}");
        }
    }

    #[tokio::test]
    async fn clusters_from_remote_rejects_a_malformed_body() {
        let server = MockServer::start().await;
        mount_clusters_error(&server, 200, "{\"nope\":1}").await;
        let config = CliServerConfig::new(tagged_url(&server, "remote-malformed"));
        config.set_server_token("tok".to_string()).unwrap();
        assert!(matches!(
            config.clusters_from_remote().await,
            Err(ProxyAuthK8sError::RemoteServerError(msg)) if msg.contains("Serialization")
        ));
    }

    #[tokio::test]
    async fn clusters_from_remote_reports_an_unreachable_server() {
        // Port 9 (discard) is closed on loopback: the connection is refused.
        let config = CliServerConfig::new("http://127.0.0.1:9/remote-unreachable".to_string());
        config.set_server_token("tok".to_string()).unwrap();
        assert!(matches!(
            config.clusters_from_remote().await,
            Err(ProxyAuthK8sError::RemoteServerError(_))
        ));
    }
}
