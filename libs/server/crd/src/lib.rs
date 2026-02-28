use std::sync::Arc;

use authentication_configuration::AuthenticationConfiguration;
use base64::{prelude::BASE64_STANDARD, Engine};
use certificate::CertSource;
use common::{traits::ObjectRedis, State};
use default::default_enabled;
use kube::{config::Kubeconfig, Client, CustomResource, ResourceExt};
use reqwest::Url;
use schemars::JsonSchema;
use security::SecurityConfiguration;
use serde::{Deserialize, Serialize};
use service::Service;
use status::ProxyKubeApiStatus;
use tracing::instrument;

pub mod authentication_configuration;
pub mod certificate;
pub mod default;
pub mod security;
pub mod service;
pub mod status;

pub static PROXY_KUBE_FINALIZER: &str = "weebo.si.rs";

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "weebo.si.rs",
    version = "v1",
    kind = "ProxyKubeApi",
    plural = "proxykubeapis",
    namespaced
)]
#[kube(status = "ProxyKubeApiStatus")]
pub struct ProxyKubeApiSpec {
    /// Enable or disable the proxy
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Certificate for the Kubernetes API
    pub cert: CertSource,
    /// Service to expose the proxy
    pub service: Service,
    /// Main configuration for authentication
    pub auth_config: Option<AuthenticationConfiguration>,
    /// Security configuration
    pub security_config: Option<SecurityConfiguration>,
    /// If the proxy exposition should be accessible via the Dashboard
    /// Default: false
    #[serde(default = "default_enabled")]
    pub expose_via_dashboard: bool,
    /// If the proxy exposition is accessible via the dashboard
    /// the oidc group that allow access to the dashboard, should be unique
    /// Default: to the resource namespace + resource name
    pub dashboard_group: Option<String>,
}

impl ProxyKubeApi {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec.enabled {
            self.spec
                .auth_config
                .as_ref()
                .map_or(Ok(()), |auth_config| auth_config.validate())?;
            self.spec
                .security_config
                .as_ref()
                .map_or(Ok(()), |security_config| security_config.validate())?;
        }
        Ok(())
    }
    pub fn to_identifier(&self) -> String {
        format!(
            "proxyk8sauth:{}/{}",
            self.namespace().unwrap_or_default(),
            self.name_any()
        )
    }
    pub fn to_path(&self) -> String {
        format!(
            "{}/{}",
            self.namespace().unwrap_or_default(),
            self.name_any()
        )
    }
    pub fn to_full_path(&self, state: Arc<State>) -> String {
        format!(
            "{}/clusters/{}",
            state.oidc_cluster_redirect_base_url,
            self.to_path()
        )
    }
    pub fn get_dashboard_group(&self) -> String {
        match &self.spec.dashboard_group {
            Some(group) => group.clone(),
            None => format!(
                "dashboard-{}-{}",
                self.namespace().unwrap_or_default(),
                self.name_any()
            ),
        }
    }
    pub fn is_user_allowed(&self, user_groups: &[String]) -> bool {
        let dashboard_group = self.get_dashboard_group();
        if !self.spec.expose_via_dashboard {
            return false;
        }
        user_groups.iter().any(|g| g == &dashboard_group)
    }

    /// Check if the service is reachable
    #[instrument(skip(self, ctx))]
    pub async fn is_reachable(&self, ctx: Arc<State>) -> Result<bool, String> {
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
                Err(err.to_string())
            }
        }
    }
    pub async fn get_client(&self, ctx: Arc<State>) -> Result<reqwest::Client, String> {
        let mut reqwest_client = reqwest::ClientBuilder::new();
        reqwest_client = match &self
            .spec
            .cert
            .get_cert(ctx.client.clone(), &self.namespace().unwrap_or_default())
            .await
        {
            Ok(Some(cert)) => match reqwest::Certificate::from_pem(cert.as_bytes()) {
                Ok(c) => reqwest_client.add_root_certificate(c),
                Err(err) => return Err(err.to_string()),
            },
            Ok(None) => reqwest_client,
            Err(err) => {
                return Err(err.to_string());
            }
        };
        reqwest_client = reqwest_client.use_rustls_tls();
        match reqwest_client.build() {
            Ok(c) => Ok(c),
            Err(err) => Err(err.to_string()),
        }
    }
    pub fn get_redirect_oidc_url(
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
            return format!("{}/auth/callback/{}", kubectl_redirect, self.to_path());
        }
        format!(
            "{}/clusters/{}/auth/callback",
            state.oidc_cluster_redirect_base_url,
            self.to_path()
        )
    }
    pub fn get_oidc_conf(
        &self,
        state: Arc<State>,
        redirect_front: bool,
        redirect_kubectl: Option<String>,
    ) -> Option<common::oidc_conf::OidcConf> {
        if let Some(redirect_kubectl_uri) = redirect_kubectl.clone() {
            // Validate the redirect uri
            // The uri need to be have no path, no query and no fragment and uri should be localhost
            let parsed_uri = match Url::parse(&redirect_kubectl_uri) {
                Ok(uri) => uri,
                Err(_) => {
                    tracing::error!("Invalid redirect uri: {}", redirect_kubectl_uri);
                    return None;
                }
            };
            if parsed_uri.path() != "/"
                || parsed_uri.query().is_some()
                || parsed_uri.fragment().is_some()
                || parsed_uri.host_str().is_none()
                || parsed_uri.host_str().unwrap() != "localhost"
            {
                tracing::error!("Invalid redirect uri: {}", redirect_kubectl_uri);
                return None;
            }
            tracing::info!("Valid redirect uri: {}", redirect_kubectl_uri);
        }
        match &self.spec.auth_config {
            Some(auth_config) => {
                if auth_config.oidc_provider.enabled {
                    return Some(common::oidc_conf::OidcConf {
                        client_id: auth_config.oidc_provider.client_id.clone(),
                        client_secret: auth_config.oidc_provider.client_secret.clone(),
                        issuer_url: auth_config.oidc_provider.issuer_url.clone(),
                        scopes: auth_config.oidc_provider.extra_scope.clone(),
                        audience: auth_config.oidc_provider.client_id.clone(),
                        redirect_url: Some(self.get_redirect_oidc_url(
                            state,
                            redirect_front,
                            redirect_kubectl,
                        )),
                    });
                }
                None
            }
            None => None,
        }
    }

    #[instrument(skip(self, state, token))]
    pub async fn to_kubeconfig(
        &self,
        state: Arc<State>,
        default_ns: Option<String>,
        token: Option<String>,
    ) -> Result<Kubeconfig, String> {
        let cluster_url = match self
            .spec
            .service
            .url_to_call(state.client.clone(), self.namespace().unwrap_or_default())
            .await
        {
            Ok(url) => url,
            Err(e) => return Err(format!("Error getting cluster URL: {}", e)),
        };
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
        });
        kubeconfig.auth_infos.push(kube::config::NamedAuthInfo {
            name: self.name_any(),
            auth_info: Some(kube::config::AuthInfo {
                token: token
                    .as_ref()
                    .map(|t| secrecy::SecretBox::new(t.clone().into())),
                ..Default::default()
            }),
        });
        kubeconfig.contexts.push(kube::config::NamedContext {
            name: self.name_any(),
            context: Some(kube::config::Context {
                cluster: self.name_any(),
                user: Some(self.name_any()),
                namespace: default_ns,
                ..Default::default()
            }),
        });
        kubeconfig.current_context = Some(self.name_any());
        Ok(kubeconfig)
    }

    #[instrument(skip(self, state, token))]
    pub async fn to_kube_client(
        &self,
        state: Arc<State>,
        default_ns: Option<String>,
        token: Option<String>,
    ) -> Result<kube::Client, String> {
        let kubeconfig = self
            .to_kubeconfig(state.clone(), default_ns.clone(), token.clone())
            .await?;

        Client::try_from(kubeconfig).map_err(|e| e.to_string())
    }
}

impl ObjectRedis for ProxyKubeApi {
    fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}
