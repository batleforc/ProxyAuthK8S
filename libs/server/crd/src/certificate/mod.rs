use base64::{Engine, prelude::BASE64_STANDARD};
use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod client_certificate;
mod error;

pub use client_certificate::ClientCertificate;
pub use error::CertError;

#[derive(Serialize, Deserialize, Clone, JsonSchema)]
pub enum CertSource {
    /// Use a cert from a secret
    Secret {
        name: String,
        key: String,
        namespace: Option<String>,
    },
    /// Base64 encoded cert
    Cert(String),
    /// Configmap
    ConfigMap {
        name: String,
        key: String,
        namespace: Option<String>,
    },
    /// Insecure, do not use TLS
    Insecure(bool),
}

/// Hand-written so the inline `Cert(_)` payload — which may hold a base64 private
/// key (a client cert's `key` is a `CertSource`) — is never written to logs when
/// a `ProxyKubeApi` is `Debug`-formatted (e.g. `debug!(proxy = ?proxy)`). The
/// Secret/ConfigMap variants only carry references (names), so they stay visible.
impl std::fmt::Debug for CertSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CertSource::Secret {
                name,
                key,
                namespace,
            } => f
                .debug_struct("Secret")
                .field("name", name)
                .field("key", key)
                .field("namespace", namespace)
                .finish(),
            CertSource::Cert(_) => f.write_str("Cert(<redacted>)"),
            CertSource::ConfigMap {
                name,
                key,
                namespace,
            } => f
                .debug_struct("ConfigMap")
                .field("name", name)
                .field("key", key)
                .field("namespace", namespace)
                .finish(),
            CertSource::Insecure(value) => f.debug_tuple("Insecure").field(value).finish(),
        }
    }
}

/// Whether a `CertSource` may read a Secret/ConfigMap from a namespace other
/// than the owning `ProxyKubeApi`'s namespace.
///
/// Defaults to `false` (secure by default): a cert read uses the controller's
/// cluster-wide `ServiceAccount`, and the resolved bytes are handed back in the
/// generated kubeconfig's `certificate_authority_data`, so allowing an arbitrary
/// `namespace` turns a tenant who can create `ProxyKubeApi` objects into a
/// cross-namespace Secret read oracle. Set `PROXYAUTH_ALLOW_CROSS_NS_CERT=true`
/// only on a single-tenant cluster where every CR author is already trusted.
///
/// This crate is schema-level and must not depend on `common`, so it reads the
/// variable itself; `common::config::Config` parses it with the same
/// [`parse_allow_cross_namespace_cert`] for visibility at startup.
fn cross_namespace_cert_allowed() -> bool {
    std::env::var(ALLOW_CROSS_NS_CERT_ENV)
        .is_ok_and(|value| parse_allow_cross_namespace_cert(&value))
}

/// Env var gating cross-namespace cert reads (`false` unless set to a truthy value).
pub const ALLOW_CROSS_NS_CERT_ENV: &str = "PROXYAUTH_ALLOW_CROSS_NS_CERT";

/// Parsing rule for `PROXYAUTH_ALLOW_CROSS_NS_CERT`: `true`/`1`/`yes`/`on`/
/// `enabled` (case-insensitive, surrounding whitespace ignored) allow it,
/// anything else denies it.
#[must_use]
pub fn parse_allow_cross_namespace_cert(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on" | "enabled"
    )
}

/// Resolve the namespace a cert is actually read from, applying the
/// cross-namespace policy above.
fn resolve_cert_namespace<'a>(requested: Option<&'a str>, cr_ns: &'a str) -> &'a str {
    match requested {
        Some(requested) if requested != cr_ns && !cross_namespace_cert_allowed() => {
            tracing::warn!(
                requested,
                cr_ns,
                "cross-namespace cert reference denied by PROXYAUTH_ALLOW_CROSS_NS_CERT=false; \
                 pinning to the resource namespace"
            );
            cr_ns
        }
        Some(requested) => requested,
        None => cr_ns,
    }
}

/// Turn a Secret value into PEM text.
///
/// The usual layout — cert-manager, `kubectl create secret tls`, a
/// `kubernetes.io/tls` Secret — stores the raw PEM (base64 only on the wire,
/// which the client already undid), so PEM is taken as-is. Older releases
/// required the value to be base64-encoded PEM on top of that; anything that is
/// not PEM is still base64-decoded so those Secrets keep working.
fn decode_secret_cert(raw: &[u8]) -> Result<String, CertError> {
    let text = String::from_utf8(raw.to_vec())?;
    if text.trim_start().starts_with("-----BEGIN") {
        return Ok(text);
    }
    let decoded = BASE64_STANDARD.decode(text.trim())?;
    Ok(String::from_utf8(decoded)?)
}

impl CertSource {
    pub async fn get_cert(&self, client: Client, ns: &str) -> Result<Option<String>, CertError> {
        match self {
            CertSource::Secret {
                name,
                key,
                namespace,
            } => {
                let target_ns = resolve_cert_namespace(namespace.as_deref(), ns);
                let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
                    kube::Api::namespaced(client, target_ns);
                let secret = secrets.get(name).await.map_err(|source| CertError::Read {
                    kind: "secret",
                    name: name.clone(),
                    source: Box::new(source),
                })?;
                if let Some(data) = secret.data {
                    if let Some(cert) = data.get(key) {
                        return decode_secret_cert(&cert.0).map(Some);
                    }
                    return Err(CertError::KeyNotFound {
                        kind: "secret",
                        key: key.clone(),
                        name: name.clone(),
                    });
                }
                if let Some(data) = secret.string_data {
                    if let Some(cert) = data.get(key) {
                        return decode_secret_cert(cert.as_bytes()).map(Some);
                    }
                    return Err(CertError::KeyNotFound {
                        kind: "secret",
                        key: key.clone(),
                        name: name.clone(),
                    });
                }
                Err(CertError::NoData {
                    kind: "secret",
                    name: name.clone(),
                })
            }
            CertSource::ConfigMap {
                name,
                key,
                namespace,
            } => {
                let target_ns = resolve_cert_namespace(namespace.as_deref(), ns);
                let configmaps: kube::Api<k8s_openapi::api::core::v1::ConfigMap> =
                    kube::Api::namespaced(client, target_ns);
                let configmap = configmaps
                    .get(name)
                    .await
                    .map_err(|source| CertError::Read {
                        kind: "configmap",
                        name: name.clone(),
                        source: Box::new(source),
                    })?;
                if let Some(data) = configmap.data {
                    if let Some(cert) = data.get(key) {
                        return Ok(Some(cert.clone()));
                    }
                    return Err(CertError::KeyNotFound {
                        kind: "configmap",
                        key: key.clone(),
                        name: name.clone(),
                    });
                }
                Err(CertError::NoData {
                    kind: "configmap",
                    name: name.clone(),
                })
            }
            CertSource::Cert(c) => {
                // base64 decode the cert
                let decoded = BASE64_STANDARD.decode(c)?;
                Ok(Some(String::from_utf8(decoded)?))
            }
            CertSource::Insecure(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";

    #[test]
    fn secret_value_holding_raw_pem_is_used_as_is() {
        assert_eq!(decode_secret_cert(PEM.as_bytes()).unwrap(), PEM);
    }

    #[test]
    fn secret_value_holding_base64_pem_is_decoded() {
        let encoded = BASE64_STANDARD.encode(PEM);
        assert_eq!(decode_secret_cert(encoded.as_bytes()).unwrap(), PEM);
        // A trailing newline (`echo | base64`) is tolerated.
        assert_eq!(
            decode_secret_cert(format!("{encoded}\n").as_bytes()).unwrap(),
            PEM
        );
    }

    #[test]
    fn secret_value_that_is_neither_pem_nor_base64_is_an_error() {
        assert!(matches!(
            decode_secret_cert(b"not a cert!"),
            Err(CertError::Base64(_))
        ));
        assert!(matches!(
            decode_secret_cert(&[0xff, 0xfe]),
            Err(CertError::Utf8(_))
        ));
    }
}
