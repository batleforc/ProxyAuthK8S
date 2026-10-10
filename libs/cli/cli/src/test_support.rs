//! Shared helpers for tests that talk to a mocked `ProxyAuthK8S` API.

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path_regex},
};

/// Clusters listing returned by [`mount_clusters`]: `team-a/prod` (SSO off)
/// and `team-a/sso` (SSO on).
pub(crate) fn clusters_body() -> serde_json::Value {
    serde_json::json!({
        "clusters": [
            {
                "enabled": true,
                "is_reachable": true,
                "name": "prod",
                "namespace": "team-a",
                "sso_enabled": false
            },
            {
                "enabled": false,
                "name": "sso",
                "namespace": "team-a",
                "sso_enabled": true
            }
        ]
    })
}

/// Base URL of a server reached through `server`, under a `tag` path prefix.
///
/// wiremock recycles mock servers (and so ports) between tests of one
/// process; the prefix keeps the keyring entries derived from the URL
/// distinct per test.
pub(crate) fn tagged_url(server: &MockServer, tag: &str) -> String {
    format!("{}/{tag}", server.uri())
}

/// Answer `GET <any prefix>/api/v1/clusters` with [`clusters_body`], only for requests
/// carrying `Bearer <token>`.
pub(crate) async fn mount_clusters(server: &MockServer, token: &str) {
    Mock::given(method("GET"))
        .and(path_regex("/api/v1/clusters$"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(clusters_body()))
        .mount(server)
        .await;
}

/// Answer `GET <any prefix>/api/v1/clusters` with `status` and a raw JSON `body`.
pub(crate) async fn mount_clusters_error(server: &MockServer, status: u16, body: &str) {
    Mock::given(method("GET"))
        .and(path_regex("/api/v1/clusters$"))
        .respond_with(
            ResponseTemplate::new(status).set_body_raw(body.to_string(), "application/json"),
        )
        .mount(server)
        .await;
}

// --- TLS --------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A generated certificate with its key.
pub(crate) struct TestCert {
    pub cert: CertificateDer<'static>,
    pub key: KeyPair,
    pub params: CertificateParams,
}

fn cert_params(cn: &str, sans: &[&str]) -> CertificateParams {
    let mut params =
        CertificateParams::new(sans.iter().map(ToString::to_string).collect::<Vec<_>>())
            .expect("params");
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, cn);
    params
}

/// A self-signed (non-CA) server certificate for `sans`.
pub(crate) fn self_signed_cert(cn: &str, sans: &[&str]) -> TestCert {
    let params = cert_params(cn, sans);
    let key = KeyPair::generate().expect("key");
    let cert = params.self_signed(&key).expect("self-signed");
    TestCert {
        cert: cert.der().clone(),
        key,
        params,
    }
}

/// A self-signed CA.
pub(crate) fn ca_cert(cn: &str) -> TestCert {
    let mut params = cert_params(cn, &[]);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let key = KeyPair::generate().expect("key");
    let cert = params.self_signed(&key).expect("self-signed CA");
    TestCert {
        cert: cert.der().clone(),
        key,
        params,
    }
}

/// A server certificate for `sans` signed by `ca`.
pub(crate) fn leaf_cert(ca: &TestCert, cn: &str, sans: &[&str]) -> TestCert {
    let params = cert_params(cn, sans);
    let key = KeyPair::generate().expect("key");
    let issuer = Issuer::new(ca.params.clone(), &ca.key);
    let cert = params.signed_by(&key, &issuer).expect("signed");
    TestCert {
        cert: cert.der().clone(),
        key,
        params,
    }
}

/// The ring provider, explicit: the workspace compiles more than one.
pub(crate) fn ring_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// An HTTPS server on loopback answering every request with
/// [`clusters_body`], whose certificate can be swapped to simulate a
/// rotation behind the same URL.
pub(crate) struct TlsTestServer {
    pub port: u16,
    config: Arc<Mutex<Arc<rustls::ServerConfig>>>,
}

impl TlsTestServer {
    fn config_for(chain: &[&TestCert]) -> Arc<rustls::ServerConfig> {
        let key = PrivateKeyDer::try_from(chain[0].key.serialize_der()).expect("key");
        Arc::new(
            rustls::ServerConfig::builder_with_provider(ring_provider())
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_no_client_auth()
                .with_single_cert(chain.iter().map(|c| c.cert.clone()).collect(), key)
                .expect("server config"),
        )
    }

    /// Serve `chain` (leaf first).
    pub(crate) async fn start(chain: &[&TestCert]) -> Self {
        let config = Arc::new(Mutex::new(Self::config_for(chain)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let shared = Arc::clone(&config);
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let current = Arc::clone(&shared.lock().expect("config"));
                tokio::spawn(async move {
                    // A rejected handshake (the probe, an untrusting client)
                    // is expected.
                    let Ok(mut tls) = tokio_rustls::TlsAcceptor::from(current).accept(tcp).await
                    else {
                        return;
                    };
                    let mut buf = [0u8; 4096];
                    let _ = tls.read(&mut buf).await;
                    let body = clusters_body().to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        Self { port, config }
    }

    /// Serve `chain` from now on (a certificate rotation).
    pub(crate) fn rotate(&self, chain: &[&TestCert]) {
        *self.config.lock().expect("config") = Self::config_for(chain);
    }

    pub(crate) fn url(&self) -> String {
        format!("https://localhost:{}", self.port)
    }
}

/// `certificate_authority_data` (base64 PEM) for `cert`.
pub(crate) fn certificate_authority_data(cert: &TestCert) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    STANDARD.encode(format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        STANDARD
            .encode(cert.cert.as_ref())
            .as_bytes()
            .chunks(64)
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    ))
}
