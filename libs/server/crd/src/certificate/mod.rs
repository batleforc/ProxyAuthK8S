use base64::{prelude::BASE64_STANDARD, Engine};
use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod client_certificate;

pub use client_certificate::ClientCertificate;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
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

/// Whether a `CertSource` may read a Secret/ConfigMap from a namespace other
/// than the owning `ProxyKubeApi`'s namespace.
///
/// Defaults to `true` (the historical behaviour, non-breaking). Set
/// `PROXYAUTH_ALLOW_CROSS_NS_CERT=false` to pin every cert read to the CR's own
/// namespace, so a tenant that can create `ProxyKubeApi` objects cannot turn the
/// controller's ServiceAccount into a cross-namespace read oracle.
fn cross_namespace_cert_allowed() -> bool {
    std::env::var("PROXYAUTH_ALLOW_CROSS_NS_CERT")
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
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
    pub async fn get_cert(&self, client: Client, ns: &str) -> Result<Option<String>, String> {
        match self {
            CertSource::Secret {
                name,
                key,
                namespace,
            } => {
                let target_ns = resolve_cert_namespace(namespace.as_deref(), ns);
                let secrets: kube::Api<k8s_openapi::api::core::v1::Secret> =
                    kube::Api::namespaced(client, target_ns);
                let secret = secrets.get(name).await.map_err(|e| e.to_string())?;
                if let Some(data) = secret.data {
                    if let Some(cert) = data.get(key) {
                        let cert_str =
                            String::from_utf8(cert.0.clone()).map_err(|e| e.to_string())?;
                        // decode the cert if it's base64 encoded
                        let decoded = BASE64_STANDARD
                            .decode(cert_str)
                            .map_err(|e| e.to_string())?;
                        return Ok(Some(decoded.into_iter().map(|c| c as char).collect()));
                    } else {
                        return Err(format!("Key {} not found in secret {}", key, name));
                    }
                }
                if let Some(data) = secret.string_data {
                    if let Some(cert) = data.get(key) {
                        let decode = BASE64_STANDARD.decode(cert).map_err(|e| e.to_string())?;
                        let cert_str = String::from_utf8(decode).map_err(|e| e.to_string())?;
                        return Ok(Some(cert_str));
                    } else {
                        return Err(format!("Key {} not found in secret {}", key, name));
                    }
                }
                Err(format!("No data found in secret {}", name))
            }
            CertSource::ConfigMap {
                name,
                key,
                namespace,
            } => {
                let target_ns = resolve_cert_namespace(namespace.as_deref(), ns);
                let configmaps: kube::Api<k8s_openapi::api::core::v1::ConfigMap> =
                    kube::Api::namespaced(client, target_ns);
                let configmap = configmaps.get(name).await.map_err(|e| e.to_string())?;
                if let Some(data) = configmap.data {
                    if let Some(cert) = data.get(key) {
                        return Ok(Some(cert.clone()));
                    } else {
                        return Err(format!("Key {} not found in configmap {}", key, name));
                    }
                }
                Err(format!("No data found in configmap {}", name))
            }
            CertSource::Cert(c) => {
                // base64 decode the cert
                let decoded = BASE64_STANDARD
                    .decode(c)
                    .map_err(|_| "Failed to decode base64")?;
                match String::from_utf8(decoded) {
                    Ok(s) => Ok(Some(s)),
                    Err(e) => Err(e.to_string()),
                }
            }
            CertSource::Insecure(_) => Ok(None),
        }
    }
}
