use std::sync::Arc;
use std::time::Duration;

use authentication_configuration::AuthenticationConfiguration;
use base64::{prelude::BASE64_STANDARD, Engine};
use certificate::{CertSource, ClientCertificate};
use common::{traits::ObjectRedis, State};
use default::{default_disabled, default_empty_array, default_enabled};
use kube::{config::Kubeconfig, Client, CustomResource, ResourceExt};
use reqwest::Url;
use schemars::JsonSchema;
use security::SecurityConfiguration;
use serde::{Deserialize, Serialize};
use service::Service;
use status::ProxyKubeApiStatus;
use tracing::instrument;
use virtual_api::{enabled_kinds, VirtualApiConfiguration, VirtualApiKind};

pub mod authentication_configuration;
pub mod certificate;
pub mod default;
pub mod security;
pub mod service;
pub mod status;
pub mod virtual_api;

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
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!has(self.dashboard_group) || self.dashboard_group != ''",
        "message": "dashboard_group must not be empty when it is set",
    }),
    serde_json::json!({
        "rule": "!has(self.proxy_group) || self.proxy_group != ''",
        "message": "proxy_group must not be empty when it is set",
    }),
]))]
pub struct ProxyKubeApiSpec {
    /// Enable or disable the proxy
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Certificate for the Kubernetes API
    pub cert: CertSource,
    /// Client certificate presented to the Kubernetes API for mutual TLS
    /// Leave unset when the cluster does not require client certificates
    pub client_cert: Option<ClientCertificate>,
    /// Service to expose the proxy
    pub service: Service,
    /// Main configuration for authentication
    pub auth_config: Option<AuthenticationConfiguration>,
    /// Security configuration
    pub security_config: Option<SecurityConfiguration>,
    /// If the proxy exposition should be accessible via the Dashboard
    /// Default: false
    #[serde(default = "default_disabled")]
    pub expose_via_dashboard: bool,
    /// If the proxy exposition is accessible via the dashboard
    /// the oidc group that allow access to the dashboard, should be unique
    /// Default: to the resource namespace + resource name
    pub dashboard_group: Option<String>,
    /// The oidc group required to proxy requests to this cluster
    /// Set it to decouple proxy access from dashboard access
    /// Default: the dashboard group when expose_via_dashboard is true,
    /// otherwise no group is required
    pub proxy_group: Option<String>,
    /// Virtual APIs to expose on top of this cluster
    /// A virtual API is an API the cluster does not serve, that the proxy
    /// synthesises by translating requests and responses
    #[serde(default = "default_empty_array::<VirtualApiConfiguration>")]
    pub virtual_apis: Vec<VirtualApiConfiguration>,
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

    /// The virtual APIs enabled on this cluster, deduplicated.
    pub fn enabled_virtual_apis(&self) -> Vec<VirtualApiKind> {
        enabled_kinds(&self.spec.virtual_apis)
    }

    /// The group a user must belong to in order to proxy requests to this cluster.
    ///
    /// `None` means the cluster is not group-restricted: any successfully
    /// authenticated caller may reach it. An explicit `proxy_group` always
    /// wins; otherwise a dashboard-exposed cluster reuses its dashboard group so
    /// that listing a cluster and using it require the same membership.
    pub fn get_proxy_group(&self) -> Option<String> {
        if let Some(group) = &self.spec.proxy_group {
            return Some(group.clone());
        }
        if self.spec.expose_via_dashboard {
            return Some(self.get_dashboard_group());
        }
        None
    }

    /// Whether the proxy is restricted to a group at all.
    pub fn is_proxy_group_restricted(&self) -> bool {
        self.get_proxy_group().is_some()
    }

    /// Whether `user_groups` may proxy requests to this cluster.
    pub fn is_proxy_allowed(&self, user_groups: &[String]) -> bool {
        match self.get_proxy_group() {
            Some(proxy_group) => user_groups.iter().any(|group| group == &proxy_group),
            None => true,
        }
    }

    pub fn need_token_validation(&self) -> bool {
        if let Some(auth_config) = &self.spec.auth_config {
            !auth_config.disable_validation
        } else {
            false
        }
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
        reqwest_client = reqwest_client
            .use_rustls_tls()
            // Bound the reachability probe: without a timeout a black-holed or
            // slow target keeps a reconcile future pending indefinitely, tying up
            // a controller concurrency slot.
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10));
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
                    return Some(common::oidc_conf::OidcConf {
                        client_id: auth_config.oidc_provider.client_id.clone(),
                        client_secret: auth_config.oidc_provider.client_secret.clone(),
                        issuer_url: auth_config.oidc_provider.issuer_url.clone(),
                        scopes: auth_config.oidc_provider.extra_scope.clone(),
                        audience: auth_config.oidc_provider.client_id.clone(),
                        redirect_url,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentication_configuration::{
        oidc_provider::OidcProvider, validate_against::ValidateAgainst,
    };
    use crate::security::{AllowedPathConfiguration, AllowedPathConfigurationEnum};

    fn oidc_provider(enabled: bool) -> OidcProvider {
        OidcProvider {
            enabled,
            issuer_url: "https://issuer.example.com".to_string(),
            client_id: "proxyauthk8s".to_string(),
            client_secret: Some("secret".to_string()),
            extra_scope: "groups".to_string(),
        }
    }

    fn auth_config(
        validate_against: ValidateAgainst,
        oidc_enabled: bool,
    ) -> AuthenticationConfiguration {
        AuthenticationConfiguration {
            jwt: Vec::new(),
            oidc_provider: oidc_provider(oidc_enabled),
            disable_validation: false,
            validate_against,
        }
    }

    fn spec() -> ProxyKubeApiSpec {
        ProxyKubeApiSpec {
            enabled: true,
            cert: CertSource::Insecure(true),
            client_cert: None,
            service: service::Service::ExternalService {
                url: "https://cluster.example.com:6443".to_string(),
            },
            auth_config: None,
            security_config: None,
            expose_via_dashboard: false,
            dashboard_group: None,
            proxy_group: None,
            virtual_apis: Vec::new(),
        }
    }

    fn proxy(spec: ProxyKubeApiSpec) -> ProxyKubeApi {
        let mut proxy = ProxyKubeApi::new("local-sso", spec);
        proxy.metadata.namespace = Some("default".to_string());
        proxy
    }

    #[test]
    fn validate_accepts_a_minimal_spec() {
        assert!(proxy(spec()).validate().is_ok());
    }

    #[test]
    fn validate_rejects_oidc_validation_without_an_enabled_provider() {
        let mut spec = spec();
        spec.auth_config = Some(auth_config(ValidateAgainst::OidcProvider, false));
        let err = proxy(spec).validate().unwrap_err();
        assert!(err.contains("OidcProvider"), "unexpected error: {err}");
    }

    #[test]
    fn validate_accepts_oidc_validation_with_an_enabled_provider() {
        let mut spec = spec();
        spec.auth_config = Some(auth_config(ValidateAgainst::OidcProvider, true));
        assert!(proxy(spec).validate().is_ok());
    }

    #[test]
    fn validate_rejects_an_invalid_security_config() {
        let mut spec = spec();
        spec.security_config = Some(SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![AllowedPathConfigurationEnum::Path(
                AllowedPathConfiguration {
                    path: "/api/v1/namespaces/{{tenant}}/pods".to_string(),
                    parametised: true,
                },
            )],
            ..SecurityConfiguration::default()
        });
        assert!(proxy(spec).validate().is_err());
    }

    #[test]
    fn validate_skips_everything_when_the_proxy_is_disabled() {
        let mut spec = spec();
        spec.enabled = false;
        spec.auth_config = Some(auth_config(ValidateAgainst::OidcProvider, false));
        assert!(proxy(spec).validate().is_ok());
    }

    #[test]
    fn dashboard_group_defaults_to_namespace_and_name() {
        assert_eq!(
            proxy(spec()).get_dashboard_group(),
            "dashboard-default-local-sso"
        );
    }

    #[test]
    fn dashboard_group_uses_the_configured_value() {
        let mut spec = spec();
        spec.dashboard_group = Some("platform-admins".to_string());
        assert_eq!(proxy(spec).get_dashboard_group(), "platform-admins");
    }

    #[test]
    fn user_is_not_allowed_when_the_proxy_is_not_exposed() {
        let proxy = proxy(spec());
        assert!(!proxy.spec.expose_via_dashboard);
        assert!(!proxy.is_user_allowed(&["dashboard-default-local-sso".to_string()]));
    }

    #[test]
    fn user_is_allowed_when_exposed_and_in_the_dashboard_group() {
        let mut spec = spec();
        spec.expose_via_dashboard = true;
        let proxy = proxy(spec);
        assert!(proxy.is_user_allowed(&["dashboard-default-local-sso".to_string()]));
        assert!(!proxy.is_user_allowed(&["other-group".to_string()]));
        assert!(!proxy.is_user_allowed(&[]));
    }

    #[test]
    fn expose_via_dashboard_defaults_to_false_when_deserialized() {
        // Regression guard: an unset field must not expose the cluster.
        let json = serde_json::json!({
            "enabled": true,
            "cert": { "Insecure": true },
            "service": { "ExternalService": { "url": "https://cluster.example.com:6443" } }
        });
        let spec: ProxyKubeApiSpec = serde_json::from_value(json).expect("spec should deserialize");
        assert!(!spec.expose_via_dashboard);
    }

    #[test]
    fn a_plain_cluster_is_not_group_restricted() {
        let proxy = proxy(spec());
        assert_eq!(proxy.get_proxy_group(), None);
        assert!(!proxy.is_proxy_group_restricted());
        // Nothing to check against, so any authenticated caller goes through.
        assert!(proxy.is_proxy_allowed(&[]));
    }

    #[test]
    fn a_dashboard_exposed_cluster_reuses_its_dashboard_group() {
        let mut spec = spec();
        spec.expose_via_dashboard = true;
        let proxy = proxy(spec);

        assert_eq!(
            proxy.get_proxy_group().as_deref(),
            Some("dashboard-default-local-sso")
        );
        assert!(proxy.is_proxy_allowed(&["dashboard-default-local-sso".to_string()]));
        assert!(!proxy.is_proxy_allowed(&["someone-else".to_string()]));
        assert!(!proxy.is_proxy_allowed(&[]));
    }

    #[test]
    fn proxy_group_decouples_proxy_access_from_dashboard_access() {
        let mut spec = spec();
        spec.expose_via_dashboard = true;
        spec.dashboard_group = Some("viewers".to_string());
        spec.proxy_group = Some("operators".to_string());
        let proxy = proxy(spec);

        assert_eq!(proxy.get_proxy_group().as_deref(), Some("operators"));
        assert!(proxy.is_proxy_allowed(&["operators".to_string()]));
        // Seeing the cluster in the dashboard is not enough to use it.
        assert!(!proxy.is_proxy_allowed(&["viewers".to_string()]));
        assert!(proxy.is_user_allowed(&["viewers".to_string()]));
    }

    #[test]
    fn proxy_group_applies_even_without_dashboard_exposure() {
        let mut spec = spec();
        spec.proxy_group = Some("operators".to_string());
        let proxy = proxy(spec);

        assert!(proxy.is_proxy_group_restricted());
        assert!(proxy.is_proxy_allowed(&["operators".to_string()]));
        assert!(!proxy.is_proxy_allowed(&["viewers".to_string()]));
    }

    #[test]
    fn need_token_validation_follows_the_auth_config() {
        let mut spec = spec();
        assert!(!proxy(spec.clone()).need_token_validation());

        spec.auth_config = Some(auth_config(ValidateAgainst::Kubernetes, false));
        assert!(proxy(spec.clone()).need_token_validation());

        if let Some(auth_config) = spec.auth_config.as_mut() {
            auth_config.disable_validation = true;
        }
        assert!(!proxy(spec).need_token_validation());
    }
}
