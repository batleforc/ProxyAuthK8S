use base64::{prelude::BASE64_STANDARD, Engine};
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
fn cross_namespace_cert_allowed() -> bool {
    std::env::var("PROXYAUTH_ALLOW_CROSS_NS_CERT").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on" | "enabled"
        )
    })
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
                    source,
                })?;
                if let Some(data) = secret.data {
                    if let Some(cert) = data.get(key) {
                        let cert_str = String::from_utf8(cert.0.clone())?;
                        // decode the cert if it's base64 encoded
                        let decoded = BASE64_STANDARD.decode(cert_str)?;
                        return Ok(Some(decoded.into_iter().map(|c| c as char).collect()));
                    }
                    return Err(CertError::KeyNotFound {
                        kind: "secret",
                        key: key.clone(),
                        name: name.clone(),
                    });
                }
                if let Some(data) = secret.string_data {
                    if let Some(cert) = data.get(key) {
                        let decode = BASE64_STANDARD.decode(cert)?;
                        let cert_str = String::from_utf8(decode)?;
                        return Ok(Some(cert_str));
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
                let configmap = configmaps.get(name).await.map_err(|source| CertError::Read {
                    kind: "configmap",
                    name: name.clone(),
                    source,
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
