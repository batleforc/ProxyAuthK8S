//! Calls to the `ProxyAuthK8S` server through the generated `client_api`.

use client_api::{
    apis::{
        Error as ApiError,
        api_clusters_api::get_all_visible_cluster,
        auth_server_api::{ServerAuthRefreshError, server_auth_refresh},
        configuration::Configuration,
    },
    models::GetAllVisibleClusterBody,
};
use std::{fs::File, path::Path, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use reqwest::Certificate;
use tracing::debug;
use url::{Host, Url};

use crate::{
    cli_config::cli_server_config::{CliServerConfig, keyring_store::unix_now},
    error::ProxyAuthK8sError,
};

/// A server token with less validity than this is renewed before use
/// (RFC 0010 §6.4).
const REFRESH_MARGIN_SECONDS: i64 = 60;

/// How long a renewal waits for another process renewing the same session
/// before going ahead on its own (100 ms steps).
const RENEWAL_LOCK_ATTEMPTS: u32 = 100;

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

    /// The server token, renewed first through `/api/v1/auth/refresh` when it
    /// has less than [`REFRESH_MARGIN_SECONDS`] left and the SSO session holds
    /// a refresh token. A pasted token (no session) is returned as is.
    pub async fn fresh_server_token(&self) -> Result<String, ProxyAuthK8sError> {
        // Checked before the keyring is read: a refused URL never sees a token.
        require_secure_server_url(&self.url)?;
        let token = self.server_token()?;
        let Some(session) = self.server_session()? else {
            return Ok(token);
        };
        if !session.expires_within(REFRESH_MARGIN_SECONDS, unix_now()) {
            return Ok(token);
        }
        // One renewal at a time per server: a refresh token an IdP rotates
        // is spent by the first, and a reuse can revoke the whole session.
        let _lock = self.renewal_lock().await;
        let Some(session) = self.server_session()? else {
            return self.server_token();
        };
        if !session.expires_within(REFRESH_MARGIN_SECONDS, unix_now()) {
            // Another process renewed it while this one waited.
            return self.server_token();
        }
        // `/auth/refresh` is not authenticated: the expiring token stays home.
        let config = api_configuration(
            &self.url,
            None,
            http_client(self.certificate_authority_data.as_deref())?,
        );
        let tokens = server_auth_refresh(&config, &session.refresh_token)
            .await
            .map_err(|e| refresh_error(&self.url, &e))?;
        debug!("Server token renewed.");
        self.store_session_tokens(tokens, Some(session.refresh_token))
    }

    /// An exclusive lock on this server's renewal, held until dropped, or
    /// `None` when it cannot be had (a lock file another user owns, a lock
    /// held too long): the renewal then goes ahead unserialised, as before.
    async fn renewal_lock(&self) -> Option<File> {
        // One file name per server URL; two URLs sharing a name only share
        // the wait.
        let name: String = self
            .url
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let path = std::env::temp_dir().join(format!("kubectl-proxyauth-renewal-{name}.lock"));
        let file = File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .ok()?;
        for _ in 0..RENEWAL_LOCK_ATTEMPTS {
            match file.try_lock() {
                Ok(()) => return Some(file),
                Err(std::fs::TryLockError::WouldBlock) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    debug!("Renewal lock unavailable: {e}");
                    return None;
                }
            }
        }
        debug!("Renewal lock still held; renewing anyway.");
        None
    }

    pub async fn clusters_from_remote(
        &self,
    ) -> Result<GetAllVisibleClusterBody, ProxyAuthK8sError> {
        let mut config = self.base_configuration()?;
        // Renewed first, so `get` and `login clusters` outlive the 5 minutes
        // of a default Keycloak access token.
        config.bearer_access_token = Some(self.fresh_server_token().await?);
        get_all_visible_cluster(&config).await.map_err(|e| {
            debug!("Error fetching clusters from remote: {:?}", e);
            e.into()
        })
    }
}

/// Why a session renewal failed: over (log in again), throttled, or the
/// `IdP` (or the server) unreachable, which only needs a later retry.
fn refresh_error(url: &str, err: &ApiError<ServerAuthRefreshError>) -> ProxyAuthK8sError {
    let unavailable = || {
        ProxyAuthK8sError::RemoteServerError(format!(
            "could not renew the session with {url}: the identity provider is unavailable, \
             try again later"
        ))
    };
    match err {
        ApiError::ResponseError(resp) => match resp.status.as_u16() {
            400 => ProxyAuthK8sError::Unauthenticated(format!(
                "the session with {url} is over; log in again with \
                 `kubectl-proxyauth login --server-url {url}`"
            )),
            429 => ProxyAuthK8sError::RemoteServerError(format!(
                "too many session renewals on {url}, try again later"
            )),
            503 => unavailable(),
            _ => ProxyAuthK8sError::RemoteServerError(format!(
                "could not renew the session with {url}: {err}"
            )),
        },
        ApiError::Reqwest(_) | ApiError::Io(_) => unavailable(),
        ApiError::Serde(_) => ProxyAuthK8sError::RemoteServerError(format!(
            "could not renew the session with {url}: {err}"
        )),
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

    // --- session renewal (RFC 0010) ---------------------------------------

    use crate::cli_config::cli_server_config::keyring_store::ServerSession;
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{body_string_contains, method, path_regex},
    };

    /// A server at `tag` holding `old-access` with a session whose access
    /// token expires `expires_in` seconds from now.
    fn with_session(server: &MockServer, tag: &str, expires_in: i64) -> CliServerConfig {
        let config = CliServerConfig::new(tagged_url(server, tag));
        config
            .set_server_login(
                "old-access",
                Some(&ServerSession {
                    refresh_token: "old-refresh".to_string(),
                    expires_at: Some(unix_now().saturating_add(expires_in)),
                }),
            )
            .unwrap();
        config
    }

    async fn mount_refresh(server: &MockServer, response: ResponseTemplate, expected: u64) {
        Mock::given(method("POST"))
            .and(path_regex("/api/v1/auth/refresh$"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .respond_with(response)
            .expect(expected)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_token_about_to_expire_is_renewed_and_the_new_pair_stored() {
        let server = MockServer::start().await;
        mount_refresh(
            &server,
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "new-access",
                "token_type": "Bearer",
                "expires_in": 300,
                "refresh_token": "new-refresh",
            })),
            1,
        )
        .await;
        let config = with_session(&server, "refresh-ok", 10);

        assert_eq!(config.fresh_server_token().await.unwrap(), "new-access");
        assert_eq!(config.server_token().unwrap(), "new-access");
        let session = config.server_session().unwrap().unwrap();
        assert_eq!(session.refresh_token, "new-refresh");
        let left = session.expires_at.unwrap().saturating_sub(unix_now());
        assert!((290..=300).contains(&left), "{left}");
        // Valid for 300 s now: no second renewal (the mock expects one call).
        assert_eq!(config.fresh_server_token().await.unwrap(), "new-access");
    }

    /// Two `get-token` at once spend the refresh token once: the second
    /// waits for the first, then finds the session renewed.
    #[tokio::test]
    async fn concurrent_renewals_spend_the_refresh_token_once() {
        let server = MockServer::start().await;
        mount_refresh(
            &server,
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "access_token": "new-access",
                    "token_type": "Bearer",
                    "expires_in": 300,
                    "refresh_token": "new-refresh",
                }))
                .set_delay(Duration::from_millis(200)),
            1,
        )
        .await;
        let config = with_session(&server, "refresh-concurrent", 10);
        let (first, second) =
            tokio::join!(config.fresh_server_token(), config.fresh_server_token());
        assert_eq!(first.unwrap(), "new-access");
        assert_eq!(second.unwrap(), "new-access");
    }

    #[tokio::test]
    async fn a_token_with_time_left_or_without_session_is_used_as_is() {
        let server = MockServer::start().await;
        mount_refresh(&server, ResponseTemplate::new(500), 0).await;
        let config = with_session(&server, "refresh-not-yet", 120);
        assert_eq!(config.fresh_server_token().await.unwrap(), "old-access");

        // A pasted token: no session, nothing to renew it with.
        let pasted = CliServerConfig::new(tagged_url(&server, "refresh-pasted"));
        pasted.set_server_login("pasted", None).unwrap();
        assert_eq!(pasted.fresh_server_token().await.unwrap(), "pasted");
    }

    #[tokio::test]
    async fn a_refused_renewal_asks_for_a_new_login() {
        let server = MockServer::start().await;
        mount_refresh(
            &server,
            ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "invalid_grant"})),
            1,
        )
        .await;
        let config = with_session(&server, "refresh-refused", 10);

        let err = config.fresh_server_token().await.unwrap_err();
        assert!(
            matches!(&err, ProxyAuthK8sError::Unauthenticated(msg)
                if msg.contains("kubectl-proxyauth login --server-url")),
            "{err}"
        );
        // Nothing stored over the old pair.
        assert_eq!(config.server_token().unwrap(), "old-access");
    }

    #[tokio::test]
    async fn an_unavailable_idp_is_reported_without_login_advice() {
        let server = MockServer::start().await;
        mount_refresh(&server, ResponseTemplate::new(503), 1).await;
        let config = with_session(&server, "refresh-503", 10);
        let err = config.fresh_server_token().await.unwrap_err();
        assert!(
            matches!(&err, ProxyAuthK8sError::RemoteServerError(msg)
                if msg.contains("unavailable") && !msg.contains("login")),
            "{err}"
        );

        // The server itself unreachable: same message.
        let down = CliServerConfig::new("http://127.0.0.1:9/refresh-down".to_string());
        down.set_server_login(
            "old-access",
            Some(&ServerSession {
                refresh_token: "old-refresh".to_string(),
                expires_at: Some(0),
            }),
        )
        .unwrap();
        assert!(matches!(
            down.fresh_server_token().await,
            Err(ProxyAuthK8sError::RemoteServerError(msg)) if msg.contains("unavailable")
        ));
    }
}
