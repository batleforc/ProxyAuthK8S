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
