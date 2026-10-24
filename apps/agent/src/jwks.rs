//! `proxyauthk8s-agent publish-jwks` (RFC 0012 §6.5, decision 24): the
//! `jwksPublisher` CronJob copies this cluster's `/openid/v1/jwks` into a
//! ConfigMap, for External Secrets to carry to the proxy's `jwks_from`.
//!
//! It runs under its own ServiceAccount: reading the JWKS needs no grant
//! (`system:service-account-issuer-discovery`), writing needs `get` and
//! `update` on that one ConfigMap, which the chart creates. The agent itself
//! keeps no RBAC.

use serde_json::Value;

use super::store::ApiClient;

/// The ConfigMap key `jwks_from` reads.
pub const KEY: &str = "jwks.json";

/// Copy the JWKS into `AGENT_JWKS_CONFIGMAP` of `AGENT_NAMESPACE`. `false`
/// when it already held it.
///
/// # Errors
///
/// A message when the JWKS cannot be read or is not one, or the ConfigMap
/// cannot be read or written (the next run of the CronJob tries again).
pub async fn publish(lookup: impl Fn(&str) -> Option<String>) -> Result<bool, String> {
    let var = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
    let name = var("AGENT_JWKS_CONFIGMAP").ok_or("AGENT_JWKS_CONFIGMAP is required")?;
    let namespace = var("AGENT_NAMESPACE").ok_or("AGENT_NAMESPACE is required")?;
    let api = ApiClient::from_lookup(var)?;
    let failed = |reason| format!("{reason}: see the log above");

    let jwks = api
        .get(&api.url(["openid", "v1", "jwks"]))
        .await
        .map_err(failed)?;
    // Never publish what jwks_from would refuse: an empty set refuses every agent.
    let keys = serde_json::from_str::<Value>(&jwks)
        .ok()
        .and_then(|set| set.get("keys")?.as_array().map(Vec::len));
    if !keys.is_some_and(|keys| keys > 0) {
        return Err("/openid/v1/jwks is not a JWKS with keys".to_string());
    }
    if jwks.len() > crate::MAX_JWKS_BYTES {
        return Err(format!(
            "the JWKS is larger than {} bytes",
            crate::MAX_JWKS_BYTES
        ));
    }

    let url = api.url(["api", "v1", "namespaces", &namespace, "configmaps", &name]);
    let mut config_map: Value = serde_json::from_str(&api.get(&url).await.map_err(failed)?)
        .map_err(|err| err.to_string())?;
    if config_map
        .pointer(&format!("/data/{KEY}"))
        .and_then(Value::as_str)
        == Some(jwks.as_str())
    {
        return Ok(false);
    }
    config_map
        .as_object_mut()
        .ok_or("the ConfigMap is not an object")?
        .insert("data".to_string(), serde_json::json!({ KEY: jwks }));
    match api.put(&url, &config_map).await.map_err(failed)? {
        true => Ok(true),
        false => Err("the ConfigMap changed while it was written".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Json, Router};

    const JWKS: &str = r#"{"keys":[{"kty":"RSA","kid":"k1","n":"AQAB","e":"AQAB"}]}"#;

    async fn apiserver(jwks: &'static str) -> (String, Arc<Mutex<(Value, u32)>>) {
        let stored = Arc::new(Mutex::new((
            serde_json::json!({
                "apiVersion": "v1", "kind": "ConfigMap",
                "metadata": { "name": "jwks", "namespace": "agent", "resourceVersion": "1" },
            }),
            0u32,
        )));
        let app =
            Router::new()
                .route("/openid/v1/jwks", get(move || async move { jwks }))
                .route(
                    "/api/v1/namespaces/agent/configmaps/jwks",
                    get(
                        |State(stored): State<Arc<Mutex<(Value, u32)>>>| async move {
                            Json(stored.lock().unwrap().0.clone())
                        },
                    )
                    .put(
                        |State(stored): State<Arc<Mutex<(Value, u32)>>>,
                         Json(body): Json<Value>| async move {
                            let mut stored = stored.lock().unwrap();
                            *stored = (body, stored.1.saturating_add(1));
                            StatusCode::OK
                        },
                    ),
                )
                .with_state(Arc::clone(&stored));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, stored)
    }

    async fn run(api: &str, token: &std::path::Path) -> Result<bool, String> {
        let vars = [
            ("AGENT_JWKS_CONFIGMAP", "jwks".to_string()),
            ("AGENT_NAMESPACE", "agent".to_string()),
            ("AGENT_API_URL", api.to_string()),
            ("AGENT_API_TOKEN_PATH", token.display().to_string()),
        ];
        publish(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })
        .await
    }

    #[tokio::test]
    async fn the_jwks_is_published_once_and_an_empty_one_never() {
        let token = std::env::temp_dir().join(format!("agent-jwks-token-{}", std::process::id()));
        std::fs::write(&token, "api-token").unwrap();
        let (api, stored) = apiserver(JWKS).await;
        assert_eq!(run(&api, &token).await, Ok(true));
        assert_eq!(
            stored.lock().unwrap().0.pointer("/data/jwks.json"),
            Some(&Value::from(JWKS))
        );
        assert_eq!(
            stored.lock().unwrap().0.pointer("/metadata/name"),
            Some(&Value::from("jwks"))
        );
        // Unchanged: no write.
        assert_eq!(run(&api, &token).await, Ok(false));
        assert_eq!(stored.lock().unwrap().1, 1);

        let (empty, stored) = apiserver(r#"{"keys":[]}"#).await;
        assert!(run(&empty, &token).await.is_err());
        assert_eq!(stored.lock().unwrap().1, 0);
        drop(std::fs::remove_file(&token));
    }
}
