//! `OneShotToken` (RFC 0012 §6.5, decision 21): the identity the agent's
//! replicas share, in the Secret `identitySecret` of its namespace, which the
//! chart creates empty and the agent may only `get` and `update`.
//!
//! The first replica to find it empty spends the token and writes key,
//! certificate and agent CA; the others read them. Every write carries the
//! `resourceVersion` it read: the loser of a race reads the winner's. A
//! restarted pod or an added replica spends no token.

use std::path::PathBuf;

use agent_pki::AgentKey;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::{Method, StatusCode, Url};
use serde_json::{Value, json};
use tracing::warn;

use super::{Identity, Reason, STEP_TIMEOUT};

const KEY: &str = "tls.key";
const CERT: &str = "tls.crt";
const CA: &str = "ca.crt";

/// The local apiserver, as the agent's own ServiceAccount.
#[derive(Clone, Debug)]
pub(super) struct ApiClient {
    base: Url,
    /// The agent's API token, projected and rotated by the kubelet: read at
    /// each call.
    token_path: PathBuf,
    http: reqwest::Client,
}

impl ApiClient {
    /// From `AGENT_API_URL` (default: `KUBERNETES_SERVICE_HOST` and `_PORT`),
    /// `AGENT_API_TOKEN_PATH` and the CA at `AGENT_KUBE_CA_PATH`.
    pub(super) fn from_lookup(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let api = match var("AGENT_API_URL") {
            Some(url) => url,
            None => {
                let host = var("KUBERNETES_SERVICE_HOST")
                    .ok_or("AGENT_API_URL or KUBERNETES_SERVICE_HOST is required")?;
                let port = var("KUBERNETES_SERVICE_PORT").unwrap_or_else(|| "443".to_string());
                if host.contains(':') {
                    format!("https://[{host}]:{port}")
                } else {
                    format!("https://{host}:{port}")
                }
            }
        };
        let base: Url = api.parse().map_err(|err| format!("AGENT_API_URL: {err}"))?;
        if base.cannot_be_a_base() {
            return Err("AGENT_API_URL cannot be a base".to_string());
        }
        // The local apiserver: never through the egress proxy.
        let mut http = reqwest::Client::builder().no_proxy().timeout(STEP_TIMEOUT);
        if let Some(path) = var("AGENT_KUBE_CA_PATH") {
            let pem =
                std::fs::read(&path).map_err(|err| format!("AGENT_KUBE_CA_PATH {path}: {err}"))?;
            http = http.add_root_certificate(
                reqwest::Certificate::from_pem(&pem)
                    .map_err(|err| format!("AGENT_KUBE_CA_PATH {path}: {err}"))?,
            );
        }
        Ok(Self {
            base,
            token_path: var("AGENT_API_TOKEN_PATH")
                .unwrap_or_else(|| "/var/run/secrets/proxyauth/api-token".to_string())
                .into(),
            http: http.build().map_err(|err| err.to_string())?,
        })
    }

    /// `segments` under the API's base URL.
    pub(super) fn url<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Url {
        let mut url = self.base.clone();
        // `from_lookup` refused a base that cannot hold a path.
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }

    async fn call(
        &self,
        method: Method,
        url: &Url,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, Reason> {
        let token = tokio::fs::read_to_string(&self.token_path)
            .await
            .map_err(|err| {
                warn!(error = %err, path = %self.token_path.display(), "no API token");
                "enroll"
            })?;
        let mut request = self
            .http
            .request(method, url.clone())
            .bearer_auth(token.trim());
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().await.map_err(|err| {
            warn!(error = %err, %url, "local apiserver unreachable");
            "enroll"
        })
    }

    /// The body of a `GET`.
    pub(super) async fn get(&self, url: &Url) -> Result<String, Reason> {
        let response = self.call(Method::GET, url, None).await?;
        if !response.status().is_success() {
            warn!(status = %response.status(), %url, "unreadable");
            return Err("enroll");
        }
        response.text().await.map_err(|_| "enroll")
    }

    /// `PUT` `object` as read, its `resourceVersion` included. `false` when
    /// it was written since.
    pub(super) async fn put(&self, url: &Url, object: &Value) -> Result<bool, Reason> {
        let response = self.call(Method::PUT, url, Some(object)).await?;
        match response.status() {
            status if status.is_success() => Ok(true),
            StatusCode::CONFLICT => Ok(false),
            status => {
                warn!(%status, %url, "not written");
                Err("enroll")
            }
        }
    }
}

/// The identity Secret.
#[derive(Clone, Debug)]
pub struct IdentityStore {
    api: ApiClient,
    /// `…/api/v1/namespaces/<namespace>/secrets/<name>`.
    url: Url,
}

impl IdentityStore {
    /// From `AGENT_IDENTITY_SECRET`, `AGENT_NAMESPACE` and [`ApiClient`]'s.
    pub(super) fn from_lookup(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let name = var("AGENT_IDENTITY_SECRET").ok_or("AGENT_IDENTITY_SECRET is required")?;
        let namespace = var("AGENT_NAMESPACE").ok_or("AGENT_NAMESPACE is required")?;
        let api = ApiClient::from_lookup(var)?;
        let url = api.url(["api", "v1", "namespaces", &namespace, "secrets", &name]);
        Ok(Self { api, url })
    }

    /// The Secret as read, and the identity it holds, if any.
    pub(super) async fn load(&self) -> Result<(Value, Option<Identity>), Reason> {
        let secret: Value =
            serde_json::from_str(&self.api.get(&self.url).await?).map_err(|_| "enroll")?;
        let identity = identity_in(&secret);
        Ok((secret, identity))
    }

    /// Write `data` into `secret`, as [`Self::load`] read it (its labels and
    /// annotations stay). `false` when another replica wrote it since.
    pub(super) async fn save(&self, mut secret: Value, data: Value) -> Result<bool, Reason> {
        secret
            .as_object_mut()
            .ok_or("enroll")?
            .insert("data".to_string(), data);
        self.api.put(&self.url, &secret).await
    }
}

impl Identity {
    /// The Secret's `data` holding this identity.
    pub(super) fn to_data(&self) -> Value {
        json!({
            KEY: STANDARD.encode(&self.key_pem),
            CERT: STANDARD.encode(&self.cert_pem),
            CA: STANDARD.encode(&self.ca_pem),
        })
    }
}

/// The identity a Secret holds; `None` when empty or unreadable.
fn identity_in(secret: &Value) -> Option<Identity> {
    let field = |key: &str| {
        let bytes = STANDARD
            .decode(secret.get("data")?.get(key)?.as_str()?)
            .ok()?;
        String::from_utf8(bytes).ok()
    };
    let cert_pem = field(CERT)?;
    let key_pem = field(KEY)?;
    Some(Identity {
        key: Some(AgentKey::from_pem(&key_pem).ok()?),
        key_pem,
        not_after: Identity::not_after_of(&cert_pem).ok()?,
        cert_pem,
        ca_pem: field(CA)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use agent_pki::{AgentCa, AgentIdentity};
    use axum::extract::State;
    use axum::routing::get;
    use axum::{Json, Router};

    /// An apiserver holding one Secret, refusing a write whose
    /// `resourceVersion` is not the current one.
    async fn apiserver(secret: Value) -> (Url, Arc<Mutex<Value>>) {
        let stored = Arc::new(Mutex::new(secret));
        let app = Router::new()
            .route(
                "/api/v1/namespaces/agent/secrets/identity",
                get(|State(stored): State<Arc<Mutex<Value>>>| async move {
                    Json(stored.lock().unwrap().clone())
                })
                .put(
                    |State(stored): State<Arc<Mutex<Value>>>, Json(mut body): Json<Value>| async move {
                        let mut current = stored.lock().unwrap();
                        let version = &current["metadata"]["resourceVersion"];
                        if body["metadata"]["resourceVersion"] != *version {
                            return StatusCode::CONFLICT;
                        }
                        let next = version.as_str().unwrap().parse::<u64>().unwrap().saturating_add(1);
                        body["metadata"]["resourceVersion"] = json!(next.to_string());
                        *current = body;
                        StatusCode::OK
                    },
                ),
            )
            .with_state(Arc::clone(&stored));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, stored)
    }

    fn store(api: &Url, token_path: &std::path::Path) -> IdentityStore {
        let vars = [
            ("AGENT_IDENTITY_SECRET", "identity".to_string()),
            ("AGENT_NAMESPACE", "agent".to_string()),
            ("AGENT_API_URL", api.to_string()),
            ("AGENT_API_TOKEN_PATH", token_path.display().to_string()),
        ];
        IdentityStore::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .unwrap()
    }

    fn identity() -> Identity {
        let ca = AgentCa::generate().unwrap();
        let key = AgentKey::generate().unwrap();
        let cert_pem = ca
            .issue_agent(
                &key.csr_pem().unwrap(),
                &AgentIdentity {
                    namespace: "team-a".into(),
                    name: "prod".into(),
                    uid: "6c1f3a52-0b8e-4b7f-9d2a-3e4f5a6b7c8d".into(),
                    epoch: 0,
                },
            )
            .unwrap();
        Identity {
            not_after: Identity::not_after_of(&cert_pem).unwrap(),
            key_pem: key.key_pem(),
            key: Some(key),
            cert_pem,
            ca_pem: ca.cert_pem().to_string(),
        }
    }

    #[tokio::test]
    async fn the_identity_is_shared_and_a_stale_write_loses() {
        let token = std::env::temp_dir().join(format!("agent-api-token-{}", std::process::id()));
        std::fs::write(&token, "api-token").unwrap();
        let (api, stored) = apiserver(json!({
            "apiVersion": "v1", "kind": "Secret", "type": "Opaque",
            "metadata": {
                "name": "identity", "namespace": "agent", "resourceVersion": "1",
                "labels": { "app.kubernetes.io/managed-by": "Helm" },
            },
        }))
        .await;
        let store = store(&api, &token);

        let (empty, none) = store.load().await.unwrap();
        assert!(none.is_none());
        let mine = identity();
        assert!(store.save(empty.clone(), mine.to_data()).await.unwrap());
        // A second replica that read the empty Secret too: refused.
        assert!(!store.save(empty, identity().to_data()).await.unwrap());

        let (secret, shared) = store.load().await.unwrap();
        let shared = shared.expect("the written identity");
        assert_eq!(shared.key_pem, mine.key_pem);
        assert_eq!(shared.cert_pem, mine.cert_pem);
        assert_eq!(
            secret["metadata"]["labels"]["app.kubernetes.io/managed-by"],
            "Helm"
        );
        assert!(store.save(secret, json!({})).await.unwrap());
        assert!(store.load().await.unwrap().1.is_none());
        assert_eq!(stored.lock().unwrap()["metadata"]["resourceVersion"], "3");
        drop(std::fs::remove_file(&token));
    }
}
