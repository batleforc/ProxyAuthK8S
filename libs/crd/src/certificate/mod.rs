use base64::{prelude::BASE64_STANDARD, Engine};
use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// Insecure, do not use TLS
    Insecure(bool),
}

impl CertSource {
    pub async fn get_cert(&self, client: Client, ns: &str) -> Result<Option<String>, String> {
        match self {
            CertSource::Secret {
                name,
                key,
                namespace,
            } => {
                let target_ns = namespace.as_deref().unwrap_or(ns);
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
