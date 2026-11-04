use crate::cli_config::cli_server_config::CliServerConfig;
use crate::ctx::CliCtx;
use crate::error::ProxyAuthK8sError;
use crate::helper::{display_claims, rfc3339, unix_now};
use tracing::debug;

//https://kubernetes.io/docs/reference/access-authn-authz/authentication/#input-and-output-formats
/// Leeway on `exp`: a client clock a little ahead must not refuse a token the
/// API server still accepts.
const CLOCK_SKEW_SECS: i64 = 60;

impl CliCtx {
    /// The token kubectl sends to `namespace/cluster`, and when it expires
    /// (Unix seconds, if known): the server token, renewed when about to
    /// expire, for a cluster recorded in token exchange by `login` (RFC 0010);
    /// else the cluster token from the keyring (the OIDC `id_token`).
    ///
    /// An already expired token is refused rather than handed to kubectl,
    /// which would only get a 401 for it: the error says which login to run.
    async fn cluster_exec_token(
        server_config: &CliServerConfig,
        namespace: &str,
        cluster_name: &str,
    ) -> Result<(String, Option<i64>), ProxyAuthK8sError> {
        let token_exchange = server_config
            .get_clusters_from_ns_name(Some(namespace.to_string()), cluster_name.to_string())
            .is_some_and(|cluster| cluster.token_exchange);
        let (token, expires_at, login) = if token_exchange {
            let token = server_config.fresh_server_token().await.inspect_err(|e| {
                debug!(
                    "Failed to retrieve the server token for cluster '{}/{}': {}",
                    namespace, cluster_name, e
                );
            })?;
            // The IdP's own expiry of a session, else the token's.
            let expires_at = server_config
                .server_session()
                .ok()
                .flatten()
                .and_then(|session| session.expires_at)
                .or_else(|| display_claims(&token).exp);
            let login = format!("kubectl proxyauth login --server-url {}", server_config.url);
            (token, expires_at, login)
        } else {
            let token = server_config
                .get_cluster_token(namespace.to_string(), cluster_name.to_string())
                .inspect_err(|e| {
                    debug!(
                        "Failed to retrieve token for cluster '{}/{}': {}. Log in with 'kubectl proxyauth login {}'.",
                        namespace, cluster_name, e, cluster_name
                    );
                })?;
            let expires_at = display_claims(&token).exp;
            let login = format!("kubectl proxyauth -n {namespace} login {cluster_name}");
            (token, expires_at, login)
        };
        if expires_at.is_some_and(|at| at.saturating_add(CLOCK_SKEW_SECS) <= unix_now()) {
            return Err(ProxyAuthK8sError::Unauthenticated(format!(
                "the login to cluster '{namespace}/{cluster_name}' has expired; run `{login}`"
            )));
        }
        Ok((token, expires_at))
    }

    /// Returns `Err` on any failure so the caller can propagate a non-zero
    /// exit code. `kubectl` treats a zero exit from an exec-credential plugin as
    /// "credentials produced"; exiting 0 on failure would make it retry/fail with
    /// a confusing error instead of surfacing our message.
    pub async fn handle_get_token(
        &mut self,
        cluster_name: Option<String>,
    ) -> Result<(), ProxyAuthK8sError> {
        // KUBERNETES_EXEC_INFO is always set by kubectl when invoking an exec plugin
        let exec_info = std::env::var("KUBERNETES_EXEC_INFO").ok();
        let credential = self
            .exec_credential(cluster_name, exec_info.as_deref())
            .await?;
        // Output ExecCredential JSON to stdout (kubectl exec plugin protocol)
        println!("{credential}");
        Ok(())
    }

    /// The `ExecCredential` JSON answering kubectl's `exec_info` (the content
    /// of `KUBERNETES_EXEC_INFO`) for `cluster_name`, or the one named in it.
    async fn exec_credential(
        &self,
        cluster_name: Option<String>,
        exec_info: Option<&str>,
    ) -> Result<String, ProxyAuthK8sError> {
        debug!("Handling get token for cluster: {:?}", cluster_name);

        // if server_url is not provided and none exist in config, return error
        if self.server_url.is_empty() && self.config.default_server_name.is_empty() {
            return Err(ProxyAuthK8sError::NotLoggedIn);
        }

        let exec_ctx = if let Some(info) = exec_info {
            info
        } else {
            debug!("KUBERNETES_EXEC_INFO environment variable is not set. Cannot retrieve ctx.");
            return Err(ProxyAuthK8sError::ExecCredential(
                "KUBERNETES_EXEC_INFO environment variable is not set".to_owned(),
            ));
        };
        debug!("KUBERNETES_EXEC_INFO: {}", exec_ctx);

        let exec_info: serde_json::Value = match serde_json::from_str(exec_ctx) {
            Ok(info) => info,
            Err(e) => {
                debug!("Failed to parse KUBERNETES_EXEC_INFO as JSON: {}", e);
                return Err(ProxyAuthK8sError::ExecCredential(format!(
                    "failed to parse KUBERNETES_EXEC_INFO as JSON: {e}"
                )));
            }
        };

        // Use the apiVersion from exec info for the ExecCredential response
        let api_version = exec_info
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("client.authentication.k8s.io/v1beta1");
        debug!("API Version from exec info: {}", api_version);

        // Determine cluster_name and a candidate namespace from exec info:
        // - If cluster_name is provided as CLI arg, use it directly.
        // - Otherwise, extract both from spec.cluster.server in KUBERNETES_EXEC_INFO
        //   (URL format: https://<host>/clusters/<namespace>/<cluster-name>)
        let (cluster_name, exec_namespace) = if let Some(name) = cluster_name {
            debug!("Cluster name provided as argument: {}", name);
            let exec_ns = exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("server"))
                .and_then(|v| v.as_str())
                .and_then(|url| {
                    crate::cli_config::CliConfig::proxy_url_to_tuple(url)
                        .ok()
                        .map(|info| info.namespace)
                });
            (name, exec_ns)
        } else {
            debug!(
                "No cluster name provided, extracting from KUBERNETES_EXEC_INFO spec.cluster.server"
            );
            let cluster_server_url = if let Some(url) = exec_info
                .get("spec")
                .and_then(|s| s.get("cluster"))
                .and_then(|s| s.get("server"))
                .and_then(|v| v.as_str())
            {
                url
            } else {
                debug!(
                    "Cluster name not provided and spec.cluster.server not found in KUBERNETES_EXEC_INFO. Please provide the cluster name as argument."
                );
                return Err(ProxyAuthK8sError::ExecCredential(
                    "cluster name not provided and spec.cluster.server not found in KUBERNETES_EXEC_INFO"
                        .to_owned(),
                ));
            };
            match crate::cli_config::CliConfig::proxy_url_to_tuple(cluster_server_url) {
                Ok(url_info) => {
                    debug!(
                        "Extracted cluster '{}' in namespace '{}' from server URL",
                        url_info.cluster_name, url_info.namespace
                    );
                    (url_info.cluster_name, Some(url_info.namespace))
                }
                Err(e) => {
                    debug!(
                        "Failed to extract cluster name from server URL '{}': {}. Please provide the cluster name as argument.",
                        cluster_server_url, e
                    );
                    return Err(e.into());
                }
            }
        };

        // Resolve server config (via --server-url arg or default)
        let server_config = match self.config.get_server_config_by_url(
            if self.server_url.is_empty() {
                None
            } else {
                Some(self.server_url.clone())
            },
        ) {
            Ok(config) => config,
            Err(e) => {
                debug!(
                    "Error retrieving server configuration: {}. Please login to the server first.",
                    e
                );
                return Err(e.into());
            }
        };

        // Determine namespace: CLI arg > exec info > server default
        let namespace = if !self.namespace.is_empty() {
            self.namespace.clone()
        } else if let Some(ns) = exec_namespace {
            ns
        } else {
            server_config.namespace.clone()
        };

        debug!(
            "Retrieving token for cluster '{}/{}' from server '{}'",
            namespace, cluster_name, server_config.url
        );

        let (token, expires_at) =
            Self::cluster_exec_token(server_config, &namespace, &cluster_name).await?;

        debug!(
            "Successfully retrieved token for cluster '{}/{}'.",
            namespace, cluster_name
        );
        // get v1 or v1beta1 from api_version string (default to v1beta1 if not found)
        let api_version = if api_version.contains("v1beta1") {
            "client.authentication.k8s.io/v1beta1"
        } else {
            "client.authentication.k8s.io/v1"
        };

        let mut status = serde_json::Map::new();
        status.insert("token".to_string(), token.into());
        // kubectl caches the credential until then and runs get-token again
        // after: long-lived commands (port-forward, exec, --watch) pick up a
        // renewed token, or the login error, instead of a 401.
        if let Some(at) = expires_at.and_then(rfc3339) {
            status.insert("expirationTimestamp".to_string(), at.into());
        }
        let exec_credential = serde_json::json!({
            "apiVersion": api_version,
            "kind": "ExecCredential",
            "status": status,
        });

        match serde_json::to_string(&exec_credential) {
            Ok(output) => Ok(output),
            Err(e) => {
                debug!("Failed to serialize ExecCredential response: {}", e);
                Err(ProxyAuthK8sError::ExecCredential(format!(
                    "failed to serialize ExecCredential response: {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::cli_cluster_config::CliClusterConfig;

    #[tokio::test]
    async fn an_exchange_cluster_gets_the_server_token_never_a_cluster_token() {
        let mut server = CliServerConfig::new("http://127.0.0.1:9/get-token-exchange".to_string());
        server.set_server_token("srv-tok".to_string()).unwrap();
        // A stale cluster token is there, and must not be the one handed out.
        server
            .set_cluster_token("team-a".to_string(), "x".to_string(), "cl-tok".to_string())
            .unwrap();
        server
            .clusters
            .insert("team-a/x".to_string(), CliClusterConfig::token_exchange());

        let (token, _) = CliCtx::cluster_exec_token(&server, "team-a", "x")
            .await
            .unwrap();
        assert_eq!(token, "srv-tok");
    }

    #[tokio::test]
    async fn a_cluster_without_the_flag_keeps_its_own_token() {
        let mut server = CliServerConfig::new("http://127.0.0.1:9/get-token-plain".to_string());
        server.set_server_token("srv-tok".to_string()).unwrap();
        server
            .set_cluster_token("team-a".to_string(), "x".to_string(), "cl-tok".to_string())
            .unwrap();

        let (token, expires_at) = CliCtx::cluster_exec_token(&server, "team-a", "x")
            .await
            .unwrap();
        assert_eq!(token, "cl-tok");
        // An opaque token: no expiry known.
        assert_eq!(expires_at, None);
        assert!(matches!(
            CliCtx::cluster_exec_token(&server, "team-a", "missing").await,
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));
    }

    // --- `exec_credential`: what kubectl gets after an OIDC cluster login ---

    /// An `id_token` as the SSO cluster login stores it (opaque to get-token).
    const ID_TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJyZWFkZXIifQ.sig";

    /// A context logged in to `url` (the default server, default namespace
    /// `team-c`) whose keyring holds an id_token per `(namespace, cluster)`.
    fn ctx_with_id_tokens(url: &str, tokens: &[(&str, &str, &str)]) -> CliCtx {
        let mut ctx = CliCtx::for_test();
        let name = CliServerConfig::url_to_name_from_string(url.to_string());
        let server = ctx
            .config
            .get_or_insert_server_config(name.clone(), url.to_string());
        server.namespace = "team-c".to_string();
        for (ns, cluster, token) in tokens {
            server
                .set_cluster_token(
                    (*ns).to_string(),
                    (*cluster).to_string(),
                    (*token).to_string(),
                )
                .unwrap();
        }
        ctx.config.default_server_name = name;
        ctx
    }

    fn exec_info(api_version: &str, server: Option<&str>) -> String {
        let mut info = serde_json::json!({ "apiVersion": api_version, "kind": "ExecCredential" });
        if let Some(server) = server {
            info["spec"] = serde_json::json!({ "cluster": { "server": server } });
        }
        info.to_string()
    }

    async fn credential(
        ctx: &CliCtx,
        cluster: Option<&str>,
        info: &str,
    ) -> Result<serde_json::Value, ProxyAuthK8sError> {
        let raw = ctx
            .exec_credential(cluster.map(str::to_string), Some(info))
            .await?;
        Ok(serde_json::from_str(&raw).expect("ExecCredential is JSON"))
    }

    #[tokio::test]
    async fn kubectl_gets_the_oidc_id_token_of_the_cluster_it_targets() {
        let ctx = ctx_with_id_tokens(
            "https://gettoken-oidc.example",
            &[("team-a", "prod", ID_TOKEN)],
        );
        // No argument: the cluster and its namespace come from the proxy URL
        // kubectl is about to call.
        let out = credential(
            &ctx,
            None,
            &exec_info(
                "client.authentication.k8s.io/v1",
                Some("https://gettoken-oidc.example/clusters/team-a/prod"),
            ),
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            serde_json::json!({
                "apiVersion": "client.authentication.k8s.io/v1",
                "kind": "ExecCredential",
                "status": { "token": ID_TOKEN }
            })
        );
    }

    #[tokio::test]
    async fn the_namespace_is_the_flag_else_kubectls_url_else_the_server_default() {
        let mut ctx = ctx_with_id_tokens(
            "https://gettoken-ns.example",
            &[
                ("team-a", "prod", "tok-a"),
                ("team-b", "prod", "tok-b"),
                ("team-c", "prod", "tok-c"),
            ],
        );
        let v1 = "client.authentication.k8s.io/v1";
        let from_url = exec_info(v1, Some("https://gettoken-ns.example/clusters/team-a/prod"));
        let token = |out: serde_json::Value| out["status"]["token"].as_str().unwrap().to_string();

        // The kubeconfig written by `login` passes the cluster as argument.
        let out = credential(&ctx, Some("prod"), &from_url).await.unwrap();
        assert_eq!(token(out), "tok-a");
        // Without a usable URL, the server's default namespace.
        let out = credential(&ctx, Some("prod"), &exec_info(v1, None))
            .await
            .unwrap();
        assert_eq!(token(out), "tok-c");
        // `--namespace` (also written by `login`) wins over both.
        ctx.namespace = "team-b".to_string();
        let out = credential(&ctx, Some("prod"), &from_url).await.unwrap();
        assert_eq!(token(out), "tok-b");
    }

    #[tokio::test]
    async fn the_answer_uses_the_api_version_kubectl_asked_for() {
        let ctx = ctx_with_id_tokens(
            "https://gettoken-version.example",
            &[("team-c", "prod", ID_TOKEN)],
        );
        for (asked, answered) in [
            (
                "client.authentication.k8s.io/v1",
                "client.authentication.k8s.io/v1",
            ),
            (
                "client.authentication.k8s.io/v1beta1",
                "client.authentication.k8s.io/v1beta1",
            ),
        ] {
            let out = credential(&ctx, Some("prod"), &exec_info(asked, None))
                .await
                .unwrap();
            assert_eq!(out["apiVersion"], answered);
        }
        // No apiVersion at all: the older v1beta1, which every kubectl reads.
        let out = credential(&ctx, Some("prod"), "{}").await.unwrap();
        assert_eq!(out["apiVersion"], "client.authentication.k8s.io/v1beta1");
    }

    #[tokio::test]
    async fn get_token_outside_kubectl_or_with_a_bad_exec_info_is_refused() {
        let ctx = ctx_with_id_tokens(
            "https://gettoken-refused.example",
            &[("team-c", "prod", ID_TOKEN)],
        );
        // Not run by kubectl: no KUBERNETES_EXEC_INFO.
        assert!(matches!(
            ctx.exec_credential(Some("prod".to_string()), None).await,
            Err(ProxyAuthK8sError::ExecCredential(_))
        ));
        assert!(matches!(
            ctx.exec_credential(Some("prod".to_string()), Some("not json"))
                .await,
            Err(ProxyAuthK8sError::ExecCredential(_))
        ));
        // Neither an argument nor a cluster URL to name the cluster.
        assert!(matches!(
            credential(
                &ctx,
                None,
                &exec_info("client.authentication.k8s.io/v1", None)
            )
            .await,
            Err(ProxyAuthK8sError::ExecCredential(_))
        ));
        // A URL that is not a proxied cluster's.
        assert!(matches!(
            credential(
                &ctx,
                None,
                &exec_info(
                    "client.authentication.k8s.io/v1",
                    Some("https://apiserver:6443")
                ),
            )
            .await,
            Err(ProxyAuthK8sError::InvalidServerUrl(..))
        ));
    }

    #[tokio::test]
    async fn get_token_without_a_login_is_denied_and_never_answers_a_token() {
        let info = exec_info("client.authentication.k8s.io/v1", None);
        // Nobody logged in to any server.
        assert!(matches!(
            credential(&CliCtx::for_test(), Some("prod"), &info).await,
            Err(ProxyAuthK8sError::NotLoggedIn)
        ));

        let mut ctx = ctx_with_id_tokens(
            "https://gettoken-denied.example",
            &[("team-c", "prod", ID_TOKEN)],
        );
        // Logged in to the server, never to this cluster.
        assert!(matches!(
            credential(&ctx, Some("never-logged-in"), &info).await,
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));
        // The same cluster name in another namespace is another login.
        ctx.namespace = "team-a".to_string();
        assert!(matches!(
            credential(&ctx, Some("prod"), &info).await,
            Err(ProxyAuthK8sError::KeyringReadError(_))
        ));
        // A server never logged in to, named with --server-url.
        ctx.server_url = "https://gettoken-other.example".to_string();
        assert!(matches!(
            credential(&ctx, Some("prod"), &info).await,
            Err(ProxyAuthK8sError::ServerNotFound(_))
        ));
    }

    // --- expiry: when kubectl must ask again ---

    fn jwt_expiring_at(exp: i64) -> String {
        crate::test_support::jwt(&serde_json::json!({ "sub": "reader", "exp": exp }))
    }

    #[tokio::test]
    async fn the_credential_tells_kubectl_when_the_id_token_expires() {
        let id_token = jwt_expiring_at(4_000_000_000);
        let ctx = ctx_with_id_tokens(
            "https://gettoken-expiry.example",
            &[
                ("team-c", "prod", &id_token),
                ("team-c", "opaque", "opaque-token"),
            ],
        );
        let info = exec_info("client.authentication.k8s.io/v1", None);

        let out = credential(&ctx, Some("prod"), &info).await.unwrap();
        assert_eq!(out["status"]["token"], id_token.as_str());
        assert_eq!(out["status"]["expirationTimestamp"], "2096-10-02T07:06:40Z");
        // Nothing to read in an opaque token: no expiry, kubectl keeps it.
        let out = credential(&ctx, Some("opaque"), &info).await.unwrap();
        assert!(out["status"].get("expirationTimestamp").is_none(), "{out}");
    }

    #[tokio::test]
    async fn an_expired_id_token_is_refused_with_the_login_to_run() {
        let ctx = ctx_with_id_tokens(
            "https://gettoken-expired.example",
            &[(
                "team-c",
                "prod",
                &jwt_expiring_at(unix_now() - 2 * CLOCK_SKEW_SECS),
            )],
        );
        let err = credential(
            &ctx,
            Some("prod"),
            &exec_info("client.authentication.k8s.io/v1", None),
        )
        .await
        .unwrap_err();
        let ProxyAuthK8sError::Unauthenticated(message) = err else {
            panic!("expected Unauthenticated, got {err:?}");
        };
        assert!(
            message.contains("kubectl proxyauth -n team-c login prod"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_token_just_past_its_exp_is_still_passed_to_kubectl() {
        let id_token = jwt_expiring_at(unix_now() - CLOCK_SKEW_SECS / 2);
        let ctx = ctx_with_id_tokens(
            "https://gettoken-skew.example",
            &[("team-c", "prod", &id_token)],
        );
        let info = exec_info("client.authentication.k8s.io/v1", None);
        // A client clock up to a minute fast: the API server may still accept it.
        let out = credential(&ctx, Some("prod"), &info).await.unwrap();
        assert_eq!(out["status"]["token"], id_token.as_str());
    }

    #[tokio::test]
    async fn an_exchange_cluster_expires_with_the_sso_session() {
        use client_api::models::SessionTokens;
        let url = "https://gettoken-exchange-expiry.example";
        let mut ctx = ctx_with_id_tokens(url, &[]);
        let name = CliServerConfig::url_to_name_from_string(url.to_string());
        let server = ctx.config.servers.get_mut(&name).unwrap();
        server
            .clusters
            .insert("team-c/x".to_string(), CliClusterConfig::token_exchange());
        // An opaque access token: the expiry is the IdP's `expires_in`. An hour
        // left: used as is, no renewal (so no network call).
        let mut tokens = SessionTokens::new("opaque-access".to_string(), "Bearer".to_string());
        tokens.refresh_token = Some(Some("refresh".to_string()));
        tokens.expires_in = Some(Some(3_600));
        server.store_session_tokens(tokens, None).unwrap();
        let expires_at = server
            .server_session()
            .unwrap()
            .unwrap()
            .expires_at
            .unwrap();

        let out = credential(
            &ctx,
            Some("x"),
            &exec_info("client.authentication.k8s.io/v1", None),
        )
        .await
        .unwrap();
        assert_eq!(out["status"]["token"], "opaque-access");
        assert_eq!(
            out["status"]["expirationTimestamp"],
            rfc3339(expires_at).unwrap().as_str()
        );
    }

    #[tokio::test]
    async fn an_expired_pasted_server_token_is_refused_for_an_exchange_cluster() {
        let url = "https://gettoken-exchange-expired.example";
        let mut ctx = ctx_with_id_tokens(url, &[]);
        let name = CliServerConfig::url_to_name_from_string(url.to_string());
        let server = ctx.config.servers.get_mut(&name).unwrap();
        server
            .clusters
            .insert("team-c/x".to_string(), CliClusterConfig::token_exchange());
        // Pasted with --token: no session to renew it.
        server
            .set_server_token(jwt_expiring_at(unix_now() - 2 * CLOCK_SKEW_SECS))
            .unwrap();

        let err = credential(
            &ctx,
            Some("x"),
            &exec_info("client.authentication.k8s.io/v1", None),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, ProxyAuthK8sError::Unauthenticated(m) if m.contains(&format!("login --server-url {url}"))),
            "{err:?}"
        );
    }
}
