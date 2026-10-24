//! The agent CA's Secret (RFC 0012 §5.2, decision 26).
//!
//! Any gateway pod may find the Secret missing: each tries to `create` it, the
//! apiserver lets one through, the others get a `409` and read the winner's.
//! An existing Secret is never overwritten: a new CA would orphan every agent
//! certificate issued so far.

use std::collections::BTreeMap;

use agent_pki::{AgentCa, CA_SECRET, PkiError};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ObjectMeta, PostParams};
use tracing::info;

const CERT_KEY: &str = "tls.crt";
const KEY_KEY: &str = "tls.key";

#[derive(Debug, thiserror::Error)]
pub enum CaError {
    #[error("agent CA Secret: {0}")]
    Kube(#[from] kube::Error),
    #[error("agent CA Secret {CA_SECRET} lacks {0}")]
    Missing(&'static str),
    #[error("agent CA Secret {CA_SECRET} is not UTF-8")]
    NotText,
    #[error(transparent)]
    Pki(#[from] PkiError),
}

fn field(secret: &Secret, key: &'static str) -> Result<String, CaError> {
    let bytes = secret
        .data
        .as_ref()
        .and_then(|data| data.get(key))
        .ok_or(CaError::Missing(key))?;
    String::from_utf8(bytes.0.clone()).map_err(|_| CaError::NotText)
}

fn parse(secret: &Secret) -> Result<AgentCa, CaError> {
    Ok(AgentCa::from_pem(
        &field(secret, CERT_KEY)?,
        &field(secret, KEY_KEY)?,
    )?)
}

/// The CA of `namespace`, created on first use.
pub async fn load_or_create(client: kube::Client, namespace: &str) -> Result<AgentCa, CaError> {
    let secrets: Api<Secret> = Api::namespaced(client, namespace);
    if let Some(secret) = secrets.get_opt(CA_SECRET).await? {
        return parse(&secret);
    }
    let ca = AgentCa::generate()?;
    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(CA_SECRET.to_string()),
            namespace: Some(namespace.to_string()),
            ..ObjectMeta::default()
        },
        type_: Some("kubernetes.io/tls".to_string()),
        data: Some(BTreeMap::from([
            (
                CERT_KEY.to_string(),
                ByteString(ca.cert_pem().as_bytes().to_vec()),
            ),
            (
                KEY_KEY.to_string(),
                ByteString(ca.key_pem().as_bytes().to_vec()),
            ),
        ])),
        ..Secret::default()
    };
    match secrets.create(&PostParams::default(), &secret).await {
        Ok(_) => {
            info!(namespace, "created the agent CA");
            Ok(ca)
        }
        // Another pod won the race: its CA is the one.
        Err(kube::Error::Api(status)) if status.code == 409 => {
            parse(&secrets.get(CA_SECRET).await?)
        }
        Err(err) => Err(err.into()),
    }
}
