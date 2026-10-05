//! Runtime I/O for [`crd::ProxyKubeApi`].
//!
//! The [`crd`] crate holds the pure CRD schema so that `crdgen` can emit the
//! YAML manifests without linking Redis/reqwest/the kube client. Everything
//! that talks to the network or a live cluster lives here instead, hung off the
//! [`ProxyKubeApiRuntime`] extension trait: reachability probes, building the
//! upstream `reqwest`/kube clients, rendering a kubeconfig, and deriving the
//! OIDC configuration and redirect URLs.

use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, prelude::BASE64_STANDARD};
use common::{State, oidc_conf::OidcConf};
use crd::ProxyKubeApi;
use kube::{Client, ResourceExt, config::Kubeconfig};
use reqwest::Url;
use tracing::instrument;

/// Failure performing a runtime (network / live-cluster) operation on a
/// [`ProxyKubeApi`].
///
/// Wraps the leaf certificate/service resolution errors from [`crd`] together
/// with the HTTP and kube-client build failures that only happen at runtime, so
/// callers get a typed cause instead of an opaque string.
#[derive(Debug, thiserror::Error)]
pub enum ProxyRuntimeError {
    /// Resolving the cluster's CA or client certificate failed.
    #[error(transparent)]
    Cert(#[from] crd::certificate::CertError),
    /// Resolving the target service URL failed.
    #[error(transparent)]
    Service(#[from] crd::service::ServiceError),
    /// Building or issuing the upstream HTTP request failed.
    #[error("upstream HTTP client error: {0}")]
    Http(#[from] reqwest::Error),
    /// Building the kube client from the rendered kubeconfig failed.
    #[error("failed to build kube client: {0}")]
    KubeClient(#[from] kube::Error),
    /// The resolved CA contains no PEM `CERTIFICATE` block.
    #[error("the cluster CA contains no PEM CERTIFICATE block")]
    CaWithoutPem,
}

/// Runtime (network / live-cluster) operations on a [`ProxyKubeApi`].
///
/// Kept separate from the schema so the CRD crate stays free of `common`,
/// `reqwest`, and the kube client. Bring it into scope (`use
/// crd_runtime::ProxyKubeApiRuntime;`) to call these on a `ProxyKubeApi`.
#[allow(async_fn_in_trait)]
pub trait ProxyKubeApiRuntime {
    /// The public, dashboard-facing URL of this cluster.
    #[must_use]
    fn to_full_path(&self, state: Arc<State>) -> String;

    /// Build the OIDC `redirect_uri` for the front, kubectl, or cluster flow.
    #[must_use]
    fn get_redirect_oidc_url(
        &self,
        state: Arc<State>,
        redirect_front: bool,
        redirect_kubectl: Option<String>,
    ) -> String;

    /// Check whether the upstream service answers.
    async fn is_reachable(&self, ctx: Arc<State>) -> Result<bool, ProxyRuntimeError>;

    /// Build a `reqwest` client trusting this cluster's CA (if any).
    async fn get_client(&self, ctx: Arc<State>) -> Result<reqwest::Client, ProxyRuntimeError>;

    /// Derive the [`OidcConf`] for this cluster, if OIDC is enabled.
    fn get_oidc_conf(
        &self,
        state: Arc<State>,
        redirect_front: bool,
        redirect_kubectl: Option<String>,
    ) -> Option<OidcConf>;

    /// Derive the [`OidcConf`] used by the mediated OAuth Authorization Server
    /// flow (`/oauth/authorize`, `/oauth/callback`, `/oauth/token`).
    ///
    /// Its `redirect_url` is this proxy's own `/oauth/callback` for the
    /// cluster — distinct from the front/kubectl redirect variants — since the
    /// upstream provider must hand the code back to the proxy itself, not to
    /// an external caller.
    fn get_oauth_as_oidc_conf(&self, state: Arc<State>) -> Option<OidcConf>;

    /// Render a [`Kubeconfig`] targeting this cluster with the given token.
    async fn to_kubeconfig(
        &self,
        state: Arc<State>,
        default_ns: Option<String>,
        token: Option<String>,
    ) -> Result<Kubeconfig, ProxyRuntimeError>;

    /// Build a kube [`Client`] targeting this cluster with the given token.
    async fn to_kube_client(
        &self,
        state: Arc<State>,
        default_ns: Option<String>,
        token: Option<String>,
    ) -> Result<Client, ProxyRuntimeError>;
}

impl ProxyKubeApiRuntime for ProxyKubeApi {
    fn to_full_path(&self, state: Arc<State>) -> String {
        format!(
            "{}/clusters/{}",
            state.oidc_cluster_redirect_base_url,
            self.to_path()
        )
    }

    fn get_redirect_oidc_url(
        &self,
        state: Arc<State>,
        redirect_front: bool,
        redirect_kubectl: Option<String>,
    ) -> String {
        if redirect_front {
            return format!(
                "{}/auth/callback/{}",
                state.oidc_front_redirect_base_url,
                self.to_path()
            );
        }
        if let Some(kubectl_redirect) = redirect_kubectl {
            // The `x-kubectl-callback` header is validated to end in `/`; trim it
            // so the registered redirect URI is a clean single-slash path.
            return format!(
                "{}/auth/callback/{}",
                kubectl_redirect.trim_end_matches('/'),
                self.to_path()
            );
        }
        format!(
            "{}/clusters/{}/auth/callback",
            state.oidc_cluster_redirect_base_url,
            self.to_path()
        )
    }

    /// Check if the service is reachable
    #[instrument(skip(self, ctx))]
    async fn is_reachable(&self, ctx: Arc<State>) -> Result<bool, ProxyRuntimeError> {
        let ip = match self
            .spec
            .service
            .url_to_call(ctx.client.clone(), self.namespace().unwrap_or_default())
            .await
        {
            Ok(url) => url,
            Err(_) => return Ok(false),
        };
        let client = match self.get_client(ctx.clone()).await {
            Ok(c) => c,
            Err(err) => return Err(err),
        };
        match client.get(&ip).send().await {
            Ok(resp) => {
                if resp.status().as_u16() >= 400 && resp.status().as_u16() < 500 {
                    // client error, the service is reachable but the request is not authorized
                    return Ok(true);
                }
                if resp.status().is_success() {
                    return Ok(true);
                }
                Ok(false)
            }
            Err(err) => {
                tracing::error!(
                    "Failed to reach ProxyKubeApi {}: {}",
                    self.to_identifier(),
                    err
                );
                if let Some(status) = err.status() {
                    if status.as_u16() >= 400 && status.as_u16() < 500 {
                        // client error, the service is reachable but the request is not authorized
                        return Ok(true);
                    }
                    if status.is_success() {
                        return Ok(true);
                    }
                    return Ok(false);
                }
                Err(err.into())
            }
        }
    }

    async fn get_client(&self, ctx: Arc<State>) -> Result<reqwest::Client, ProxyRuntimeError> {
        let mut reqwest_client = reqwest::ClientBuilder::new();
        if let Some(cert) = self
            .spec
            .cert
            .get_cert(ctx.client.clone(), &self.namespace().unwrap_or_default())
            .await?
        {
            // Without this check, text with no PEM block yields zero certificates
            // under rustls and the client silently falls back to the default
            // roots: fail here, where the misconfiguration is obvious.
            if !cert.contains("-----BEGIN CERTIFICATE-----") {
                return Err(ProxyRuntimeError::CaWithoutPem);
            }
            reqwest_client = reqwest_client
                .add_root_certificate(reqwest::Certificate::from_pem(cert.as_bytes())?);
        }
        reqwest_client = reqwest_client
            .use_rustls_tls()
            // Bound the reachability probe: without a timeout a black-holed or
            // slow target keeps a reconcile future pending indefinitely, tying up
            // a controller concurrency slot.
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10));
        Ok(reqwest_client.build()?)
    }

    fn get_oidc_conf(
        &self,
        state: Arc<State>,
        redirect_front: bool,
        redirect_kubectl: Option<String>,
    ) -> Option<OidcConf> {
        if let Some(redirect_kubectl_uri) = redirect_kubectl.clone() {
            // Validate the redirect uri
            // The uri need to be have no path, no query and no fragment and uri should be localhost
            let parsed_uri = if let Ok(uri) = Url::parse(&redirect_kubectl_uri) {
                uri
            } else {
                tracing::error!("Invalid redirect uri: {}", redirect_kubectl_uri);
                return None;
            };
            if parsed_uri.path() != "/"
                || parsed_uri.query().is_some()
                || parsed_uri.fragment().is_some()
                || parsed_uri.host_str() != Some("localhost")
            {
                tracing::error!("Invalid redirect uri: {}", redirect_kubectl_uri);
                return None;
            }
            tracing::info!("Valid redirect uri: {}", redirect_kubectl_uri);
        }
        let redirect_url = if redirect_front || redirect_kubectl.is_some() {
            Some(self.get_redirect_oidc_url(state.clone(), redirect_front, redirect_kubectl))
        } else {
            None
        };
        match &self.spec.auth_config {
            Some(auth_config) => {
                if auth_config.oidc_provider.enabled {
                    let provider = &auth_config.oidc_provider;
                    // A distinct audience when configured, otherwise fall back to
                    // the client id (providers that put the client in `aud`).
                    let audience = if provider.audience.is_empty() {
                        provider.client_id.clone()
                    } else {
                        provider.audience.clone()
                    };
                    return Some(OidcConf {
                        client_id: provider.client_id.clone(),
                        client_secret: provider.client_secret.clone(),
                        issuer_url: provider.issuer_url.clone(),
                        scopes: provider.extra_scope.clone(),
                        audience,
                        accept_authorized_party: provider.accept_authorized_party,
                        redirect_url,
                    });
                }
                None
            }
            None => None,
        }
    }

    fn get_oauth_as_oidc_conf(&self, state: Arc<State>) -> Option<OidcConf> {
        let redirect_url = format!(
            "{}/clusters/{}/oauth/callback",
            state.oidc_cluster_redirect_base_url,
            self.to_path()
        );
        let mut conf = self.get_oidc_conf(state, false, None)?;
        conf.redirect_url = Some(redirect_url);
        Some(conf)
    }

    #[instrument(skip(self, state, token))]
    async fn to_kubeconfig(
        &self,
        state: Arc<State>,
        default_ns: Option<String>,
        token: Option<String>,
    ) -> Result<Kubeconfig, ProxyRuntimeError> {
        let cluster_url = self
            .spec
            .service
            .url_to_call(state.client.clone(), self.namespace().unwrap_or_default())
            .await?;
        let mut kubeconfig = Kubeconfig::default();
        kubeconfig.clusters.push(kube::config::NamedCluster {
            name: self.name_any(),
            cluster: Some(kube::config::Cluster {
                server: Some(cluster_url),
                certificate_authority_data: self
                    .spec
                    .cert
                    .get_cert(state.client.clone(), &self.namespace().unwrap_or_default())
                    .await?
                    .as_ref()
                    .map(|cert| BASE64_STANDARD.encode(cert)),
                ..Default::default()
            }),
            other: Default::default(),
        });
        kubeconfig.auth_infos.push(kube::config::NamedAuthInfo {
            name: self.name_any(),
            auth_info: Some(kube::config::AuthInfo {
                token: token
                    .as_ref()
                    .map(|t| secrecy::SecretBox::new(t.clone().into())),
                ..Default::default()
            }),
            other: Default::default(),
        });
        kubeconfig.contexts.push(kube::config::NamedContext {
            name: self.name_any(),
            context: Some(kube::config::Context {
                cluster: self.name_any(),
                user: Some(self.name_any()),
                namespace: default_ns,
                ..Default::default()
            }),
            other: Default::default(),
        });
        kubeconfig.current_context = Some(self.name_any());
        Ok(kubeconfig)
    }

    #[instrument(skip(self, state, token))]
    async fn to_kube_client(
        &self,
        state: Arc<State>,
        default_ns: Option<String>,
        token: Option<String>,
    ) -> Result<kube::Client, ProxyRuntimeError> {
        let kubeconfig = self
            .to_kubeconfig(state.clone(), default_ns.clone(), token.clone())
            .await?;

        Ok(Client::try_from(kubeconfig)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crd::{
        ProxyKubeApiSpec,
        authentication_configuration::{
            AuthenticationConfiguration, OidcProvider, ValidateAgainst,
        },
        certificate::CertSource,
        service::Service,
    };

    const NS: &str = "team-a";
    const NAME: &str = "prod";
    const UPSTREAM: &str = "https://upstream.example.com:6443";
    const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";

    /// A `State` whose kube client and Redis point at closed ports. None of the
    /// functions under test here touch either; a call that did would fail fast.
    fn state() -> Arc<State> {
        // Both rustls providers are linked in the workspace; the server picks
        // ring in `main`, so tests must too or the kube client build panics.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let kube_config = kube::Config::new("http://127.0.0.1:1".parse().expect("static uri"));
        Arc::new(State::from_parts(
            Client::try_from(kube_config).expect("kube client should build"),
            common::redis_pool::RedisPool::from_url_with_mode("redis://127.0.0.1:1", false)
                .expect("pool should build"),
            OidcConf {
                client_id: "proxyauthk8s".to_string(),
                client_secret: None,
                issuer_url: "https://idp.example.com".to_string(),
                scopes: "openid".to_string(),
                audience: "proxyauthk8s".to_string(),
                accept_authorized_party: false,
                redirect_url: None,
            },
            "https://proxy.example.com".to_string(),
            "https://front.example.com".to_string(),
        ))
    }

    fn proxy(auth_config: Option<AuthenticationConfiguration>) -> ProxyKubeApi {
        let mut proxy = ProxyKubeApi::new(
            NAME,
            ProxyKubeApiSpec {
                enabled: true,
                cert: CertSource::Insecure(true),
                client_cert: None,
                service: Service::ExternalService {
                    url: UPSTREAM.to_string(),
                },
                auth_config,
                security_config: None,
                expose_via_dashboard: false,
                dashboard_group: None,
                proxy_group: None,
                virtual_apis: Vec::new(),
            },
        );
        proxy.metadata.namespace = Some(NS.to_string());
        proxy
    }

    fn oidc(enabled: bool, audience: &str) -> AuthenticationConfiguration {
        AuthenticationConfiguration {
            jwt: Vec::new(),
            oidc_provider: OidcProvider {
                enabled,
                issuer_url: "https://cluster-idp.example.com".to_string(),
                client_id: "cluster-client".to_string(),
                client_secret: Some("cluster-secret".to_string()),
                extra_scope: "groups".to_string(),
                audience: audience.to_string(),
                accept_authorized_party: true,
                expose_oauth_authorization_server: false,
            },
            disable_validation: false,
            validate_against: ValidateAgainst::OidcProvider,
        }
    }

    fn with_oidc() -> ProxyKubeApi {
        proxy(Some(oidc(true, "")))
    }

    #[tokio::test]
    async fn full_path_is_the_cluster_route_on_the_proxy_base_url() {
        assert_eq!(
            proxy(None).to_full_path(state()),
            "https://proxy.example.com/clusters/team-a/prod"
        );
    }

    #[tokio::test]
    async fn redirect_url_defaults_to_the_proxy_cluster_callback() {
        assert_eq!(
            proxy(None).get_redirect_oidc_url(state(), false, None),
            "https://proxy.example.com/clusters/team-a/prod/auth/callback"
        );
    }

    #[tokio::test]
    async fn redirect_url_for_the_front_uses_the_front_base_url() {
        assert_eq!(
            proxy(None).get_redirect_oidc_url(state(), true, None),
            "https://front.example.com/auth/callback/team-a/prod"
        );
    }

    #[tokio::test]
    async fn redirect_url_front_wins_over_kubectl() {
        assert_eq!(
            proxy(None).get_redirect_oidc_url(
                state(),
                true,
                Some("http://localhost:8000/".to_string())
            ),
            "https://front.example.com/auth/callback/team-a/prod"
        );
    }

    #[tokio::test]
    async fn redirect_url_for_kubectl_trims_trailing_slashes() {
        let proxy = proxy(None);
        for uri in [
            "http://localhost:8000",
            "http://localhost:8000/",
            "http://localhost:8000//",
        ] {
            assert_eq!(
                proxy.get_redirect_oidc_url(state(), false, Some(uri.to_string())),
                "http://localhost:8000/auth/callback/team-a/prod",
                "kubectl callback {uri}"
            );
        }
    }

    #[tokio::test]
    async fn oidc_conf_is_none_without_an_auth_config() {
        let proxy = proxy(None);
        assert!(proxy.get_oidc_conf(state(), false, None).is_none());
        assert!(proxy.get_oidc_conf(state(), true, None).is_none());
        assert!(
            proxy
                .get_oidc_conf(state(), false, Some("http://localhost:8000/".to_string()))
                .is_none()
        );
        assert!(proxy.get_oauth_as_oidc_conf(state()).is_none());
    }

    #[tokio::test]
    async fn oidc_conf_is_none_when_the_provider_is_disabled() {
        let proxy = proxy(Some(oidc(false, "aud")));
        assert!(proxy.get_oidc_conf(state(), false, None).is_none());
        assert!(proxy.get_oidc_conf(state(), true, None).is_none());
        assert!(proxy.get_oauth_as_oidc_conf(state()).is_none());
    }

    #[tokio::test]
    async fn oidc_conf_default_mode_copies_the_provider_without_a_redirect() {
        let conf = with_oidc()
            .get_oidc_conf(state(), false, None)
            .expect("enabled provider yields a conf");
        assert_eq!(conf.client_id, "cluster-client");
        assert_eq!(conf.client_secret.as_deref(), Some("cluster-secret"));
        assert_eq!(conf.issuer_url, "https://cluster-idp.example.com");
        assert_eq!(conf.scopes, "groups");
        assert!(conf.accept_authorized_party);
        assert_eq!(conf.redirect_url, None);
    }

    #[tokio::test]
    async fn oidc_conf_audience_falls_back_to_the_client_id() {
        let conf = with_oidc().get_oidc_conf(state(), false, None).unwrap();
        assert_eq!(conf.audience, "cluster-client");

        let conf = proxy(Some(oidc(true, "kubernetes")))
            .get_oidc_conf(state(), false, None)
            .unwrap();
        assert_eq!(conf.audience, "kubernetes");
    }

    #[tokio::test]
    async fn oidc_conf_front_mode_redirects_to_the_front() {
        let conf = with_oidc().get_oidc_conf(state(), true, None).unwrap();
        assert_eq!(
            conf.redirect_url.as_deref(),
            Some("https://front.example.com/auth/callback/team-a/prod")
        );
    }

    #[tokio::test]
    async fn oidc_conf_kubectl_mode_redirects_to_the_local_callback() {
        let conf = with_oidc()
            .get_oidc_conf(state(), false, Some("http://localhost:8000/".to_string()))
            .unwrap();
        assert_eq!(
            conf.redirect_url.as_deref(),
            Some("http://localhost:8000/auth/callback/team-a/prod")
        );
    }

    #[tokio::test]
    async fn oidc_conf_rejects_a_kubectl_uri_that_is_not_a_bare_localhost_origin() {
        let proxy = with_oidc();
        for uri in [
            "not a url",
            "http://127.0.0.1:8000/",
            "http://evil.example.com/",
            "http://localhost.evil.example.com/",
            "http://localhost:8000/callback",
            "http://localhost:8000/?next=1",
            "http://localhost:8000/#frag",
        ] {
            assert!(
                proxy
                    .get_oidc_conf(state(), false, Some(uri.to_string()))
                    .is_none(),
                "{uri} must be rejected"
            );
            // The front flag does not bypass the kubectl URI validation.
            assert!(
                proxy
                    .get_oidc_conf(state(), true, Some(uri.to_string()))
                    .is_none(),
                "{uri} must be rejected even in front mode"
            );
        }
    }

    #[tokio::test]
    async fn oauth_as_conf_redirects_to_the_proxy_oauth_callback() {
        let conf = with_oidc().get_oauth_as_oidc_conf(state()).unwrap();
        assert_eq!(
            conf.redirect_url.as_deref(),
            Some("https://proxy.example.com/clusters/team-a/prod/oauth/callback")
        );
        assert_eq!(conf.client_id, "cluster-client");
    }

    #[tokio::test]
    async fn kubeconfig_names_cluster_user_and_context_after_the_proxy() {
        let kubeconfig = proxy(None)
            .to_kubeconfig(state(), Some("apps".to_string()), Some("tok".to_string()))
            .await
            .expect("external service + insecure cert needs no cluster access");

        assert_eq!(kubeconfig.current_context.as_deref(), Some(NAME));

        assert_eq!(kubeconfig.clusters.len(), 1);
        assert_eq!(kubeconfig.clusters[0].name, NAME);
        let cluster = kubeconfig.clusters[0].cluster.as_ref().unwrap();
        assert_eq!(cluster.server.as_deref(), Some(UPSTREAM));
        assert_eq!(cluster.certificate_authority_data, None);

        assert_eq!(kubeconfig.auth_infos.len(), 1);
        assert_eq!(kubeconfig.auth_infos[0].name, NAME);
        let token = kubeconfig.auth_infos[0]
            .auth_info
            .as_ref()
            .unwrap()
            .token
            .as_ref()
            .expect("token is set");
        assert_eq!(secrecy::ExposeSecret::expose_secret(token), "tok");

        assert_eq!(kubeconfig.contexts.len(), 1);
        assert_eq!(kubeconfig.contexts[0].name, NAME);
        let context = kubeconfig.contexts[0].context.as_ref().unwrap();
        assert_eq!(context.cluster, NAME);
        assert_eq!(context.user.as_deref(), Some(NAME));
        assert_eq!(context.namespace.as_deref(), Some("apps"));
    }

    #[tokio::test]
    async fn kubeconfig_without_token_or_namespace_leaves_them_unset() {
        let kubeconfig = proxy(None)
            .to_kubeconfig(state(), None, None)
            .await
            .unwrap();
        assert!(
            kubeconfig.auth_infos[0]
                .auth_info
                .as_ref()
                .unwrap()
                .token
                .is_none()
        );
        assert_eq!(
            kubeconfig.contexts[0].context.as_ref().unwrap().namespace,
            None
        );
    }

    #[tokio::test]
    async fn kubeconfig_embeds_the_inline_ca_as_base64_pem() {
        let mut proxy = proxy(None);
        proxy.spec.cert = CertSource::Cert(BASE64_STANDARD.encode(PEM));
        let kubeconfig = proxy.to_kubeconfig(state(), None, None).await.unwrap();
        let ca = kubeconfig.clusters[0]
            .cluster
            .as_ref()
            .unwrap()
            .certificate_authority_data
            .as_deref()
            .expect("inline CA is embedded");
        // kubeconfig's `certificate-authority-data` is the base64 of the PEM.
        assert_eq!(BASE64_STANDARD.decode(ca).unwrap(), PEM.as_bytes());
    }

    #[tokio::test]
    async fn kubeconfig_surfaces_an_undecodable_inline_ca() {
        let mut proxy = proxy(None);
        proxy.spec.cert = CertSource::Cert("%%% not base64 %%%".to_string());
        let err = proxy.to_kubeconfig(state(), None, None).await.unwrap_err();
        assert!(
            matches!(
                err,
                ProxyRuntimeError::Cert(crd::certificate::CertError::Base64(_))
            ),
            "{err:?}"
        );
        // `Cert` is transparent: the leaf message is shown as-is.
        assert!(
            err.to_string()
                .starts_with("failed to base64-decode certificate: "),
            "{err}"
        );
    }

    #[test]
    fn transparent_errors_show_the_leaf_message() {
        let err = ProxyRuntimeError::from(crd::certificate::CertError::NoData {
            kind: "secret",
            name: "ca".to_string(),
        });
        assert_eq!(err.to_string(), "no data found in secret ca");

        let err = ProxyRuntimeError::from(crd::service::ServiceError::NoSpec {
            name: "api".to_string(),
        });
        assert_eq!(
            err.to_string(),
            crd::service::ServiceError::NoSpec {
                name: "api".to_string()
            }
            .to_string()
        );
    }

    #[tokio::test]
    async fn kube_client_errors_are_prefixed() {
        let mut proxy = proxy(None);
        proxy.spec.service = Service::ExternalService {
            url: "not a url".to_string(),
        };
        let Err(err) = proxy.to_kube_client(state(), None, None).await else {
            panic!("an unparsable server URL cannot build a client");
        };
        assert!(matches!(err, ProxyRuntimeError::KubeClient(_)), "{err:?}");
        assert!(
            err.to_string().starts_with("failed to build kube client: "),
            "{err}"
        );
    }

    #[tokio::test]
    async fn kube_client_builds_from_a_valid_kubeconfig() {
        let built = proxy(None)
            .to_kube_client(state(), Some("apps".to_string()), Some("tok".to_string()))
            .await;
        assert!(
            built.is_ok(),
            "building the client does not dial the cluster: {:?}",
            built.err()
        );
    }
}
