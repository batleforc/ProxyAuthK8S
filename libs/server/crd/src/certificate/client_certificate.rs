use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{CertError, CertSource};

/// Client certificate presented to the target cluster for mutual TLS.
///
/// Both halves use the same sources as the server certificate, so a cert and
/// its key can live in the same Secret under two keys.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ClientCertificate {
    /// PEM client certificate chain
    pub cert: CertSource,
    /// PEM private key matching the certificate
    pub key: CertSource,
}

impl ClientCertificate {
    /// Resolve both halves, or explain which one could not be read.
    pub async fn resolve(&self, client: Client, ns: &str) -> Result<(String, String), CertError> {
        let cert = self
            .cert
            .get_cert(client.clone(), ns)
            .await?
            .ok_or(CertError::Empty {
                half: "certificate",
            })?;
        let key = self
            .key
            .get_cert(client, ns)
            .await?
            .ok_or(CertError::Empty { half: "key" })?;
        Ok((cert, key))
    }
}
