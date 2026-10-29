//! `ProxyKubeApi` custom resource definition and its domain model.
//!
//! Defines the CRD schema (consumed by `crdgen` to emit the YAML manifests)
//! together with the security, authentication, and certificate configuration
//! types that make up a proxied cluster's spec.

use authentication_configuration::AuthenticationConfiguration;
use certificate::{CertSource, ClientCertificate};
use default::{default_disabled, default_empty_array, default_enabled};
use kube::{CustomResource, ResourceExt};
use schemars::JsonSchema;
use security::SecurityConfiguration;
use serde::{Deserialize, Serialize};
use service::Service;
use status::ProxyKubeApiStatus;
use upstream_auth::UpstreamAuth;
use virtual_api::{VirtualApiConfiguration, VirtualApiKind, enabled_kinds};

pub mod authentication_configuration;
pub mod certificate;
pub mod default;
pub mod security;
pub mod service;
pub mod status;
pub mod upstream_auth;
pub mod virtual_api;

pub static PROXY_KUBE_FINALIZER: &str = "weebo.si.rs";

/// Redis key prefix under which cached [`ProxyKubeApi`] objects live.
///
/// Single source of truth shared by the controller (writer, via its Redis
/// index) and the request path (reader): both must agree on this prefix or a
/// cached proxy is written under one key and looked up under another.
pub const REDIS_PREFIX: &str = "proxyk8sauth";

/// Redis key prefix of a cluster in `FrontProxy` or `Issuer` (RFC 0014,
/// decision 42). A binary older than RFC 0014 reads [`REDIS_PREFIX`] only,
/// so it finds no such cluster and answers `404` instead of relaying the
/// caller's token over a connection the apiserver trusts.
pub const REDIS_TRUSTED_PREFIX: &str = "proxyk8sauth-upstream-auth";

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "weebo.si.rs",
    version = "v1",
    kind = "ProxyKubeApi",
    plural = "proxykubeapis",
    namespaced
)]
#[kube(status = "ProxyKubeApiStatus")]
// `READY` and `REASON` from the `Ready` condition (RFC 0011); declaring
// columns drops kubectl's default `AGE`, hence the third.
#[kube(
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type==\"Ready\")].status"}"#,
    printcolumn = r#"{"name":"Reason","type":"string","jsonPath":".status.conditions[?(@.type==\"Ready\")].reason"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#,
    // `-o wide` only: empty for a cluster that is not an `Agent` (RFC 0013).
    printcolumn = r#"{"name":"Tunnels","type":"integer","priority":1,"jsonPath":".status.agent.tunnels"}"#,
    printcolumn = r#"{"name":"Agents","type":"string","priority":1,"jsonPath":".status.agent.versions"}"#,
    // `-o wide` only: always set by the controller, empty means a controller
    // older than RFC 0014.
    printcolumn = r#"{"name":"Upstream","type":"string","priority":1,"jsonPath":".status.upstreamAuth.mode"}"#
)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!has(self.dashboard_group) || self.dashboard_group != ''",
        "message": "dashboard_group must not be empty when it is set",
    }),
    serde_json::json!({
        "rule": "!has(self.proxy_group) || self.proxy_group != ''",
        "message": "proxy_group must not be empty when it is set",
    }),
    serde_json::json!({
        "rule": "!has(self.service.Agent) || has(self.cert.ConfigMap) || has(self.cert.Secret) || has(self.cert.Cert)",
        "message": "service Agent needs the remote apiserver's CA in cert (ConfigMap, Secret or Cert), never SystemRoots",
    }),
    // RFC 0014 §4.3: the cross-field rules of the trusted modes. Each reads
    // "not a trusted mode, or …"; the rules within `upstream_auth` itself
    // live on `UpstreamAuth`.
    serde_json::json!({
        "rule": "!has(self.upstream_auth) || !has(self.upstream_auth.mode) || self.upstream_auth.mode == 'Passthrough' || !has(self.client_cert)",
        "message": "client_cert is refused in FrontProxy and Issuer: FrontProxy reads its certificate from the proxy's namespace, and in Issuer it would win over the JWT",
    }),
    serde_json::json!({
        "rule": "!has(self.upstream_auth) || !has(self.upstream_auth.mode) || self.upstream_auth.mode == 'Passthrough' || (has(self.auth_config) && !(has(self.auth_config.disable_validation) && self.auth_config.disable_validation))",
        "message": "FrontProxy and Issuer need a validated identity: set auth_config without disable_validation",
    }),
    serde_json::json!({
        "rule": "!has(self.upstream_auth) || !has(self.upstream_auth.mode) || self.upstream_auth.mode == 'Passthrough' || !has(self.auth_config) || (has(self.auth_config.validate_against) && self.auth_config.validate_against != 'Kubernetes')",
        "message": "FrontProxy and Issuer cannot validate against Kubernetes: the SelfSubjectReview needs the apiserver to know the IdP",
    }),
    serde_json::json!({
        "rule": "!has(self.upstream_auth) || !has(self.upstream_auth.mode) || self.upstream_auth.mode == 'Passthrough' || !has(self.auth_config) || !has(self.auth_config.validate_against) || self.auth_config.validate_against != 'OidcProvider' || has(self.auth_config.oidc_provider.claim_mappings)",
        "message": "FrontProxy and Issuer require oidc_provider.claim_mappings: preferred_username must not become a trusted identity",
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
    /// Default: the dashboard group when `expose_via_dashboard` is true,
    /// otherwise no group is required
    pub proxy_group: Option<String>,
    /// Virtual APIs to expose on top of this cluster
    /// A virtual API is an API the cluster does not serve, that the proxy
    /// synthesises by translating requests and responses
    #[serde(default = "default_empty_array::<VirtualApiConfiguration>")]
    pub virtual_apis: Vec<VirtualApiConfiguration>,
    /// How the proxy presents the caller's identity to the apiserver
    /// (RFC 0014). Absent: `Passthrough`, the caller's token is relayed.
    pub upstream_auth: Option<UpstreamAuth>,
}

impl ProxyKubeApi {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec.enabled {
            if let Service::Agent(agent) = &self.spec.service {
                agent.validate(&self.spec.cert)?;
            }
            self.spec.auth_config.as_ref().map_or(
                Ok(()),
                authentication_configuration::AuthenticationConfiguration::validate,
            )?;
            self.spec
                .security_config
                .as_ref()
                .map_or(Ok(()), security::SecurityConfiguration::validate)?;
            self.spec
                .virtual_apis
                .iter()
                .try_for_each(VirtualApiConfiguration::validate)?;
            if let Some(upstream) = &self.spec.upstream_auth {
                upstream.validate(
                    self.spec.client_cert.is_some(),
                    self.spec.auth_config.as_ref(),
                )?;
                self.validate_issuer_path(upstream)?;
            }
        }
        Ok(())
    }

    /// The upstream mode, `Passthrough` when the block is absent.
    #[must_use]
    pub fn upstream_mode(&self) -> upstream_auth::UpstreamAuthMode {
        self.spec
            .upstream_auth
            .as_ref()
            .map_or_else(Default::default, |upstream| upstream.mode)
    }

    /// What CEL cannot check, the namespace being out of its reach: the
    /// issuer's path is the route this proxy serves for this cluster.
    fn validate_issuer_path(&self, upstream: &UpstreamAuth) -> Result<(), String> {
        let Some(key) = &upstream.issuer_key else {
            return Ok(());
        };
        let expected = format!("/issuer/{}", self.to_path());
        if key.issuer_url.trim_end_matches('/').ends_with(&expected) {
            Ok(())
        } else {
            Err(format!(
                "issuer_key.issuer_url must end with {expected}, the route this proxy serves for this cluster"
            ))
        }
    }
    #[must_use]
    pub fn to_identifier(&self) -> String {
        format!(
            "proxyk8sauth:{}/{}",
            self.namespace().unwrap_or_default(),
            self.name_any()
        )
    }
    /// The Redis prefix this cluster is cached under (decision 42).
    #[must_use]
    pub fn redis_prefix(&self) -> &'static str {
        if self.upstream_mode().is_trusted() {
            REDIS_TRUSTED_PREFIX
        } else {
            REDIS_PREFIX
        }
    }

    /// The Redis key this cluster is cached under.
    #[must_use]
    pub fn redis_key(&self) -> String {
        format!("{}:{}", self.redis_prefix(), self.to_path())
    }

    #[must_use]
    pub fn to_path(&self) -> String {
        format!(
            "{}/{}",
            self.namespace().unwrap_or_default(),
            self.name_any()
        )
    }
    #[must_use]
    pub fn dashboard_group(&self) -> String {
        match &self.spec.dashboard_group {
            Some(group) => group.clone(),
            None => format!(
                "dashboard-{}-{}",
                self.namespace().unwrap_or_default(),
                self.name_any()
            ),
        }
    }
    #[must_use]
    pub fn is_user_allowed(&self, user_groups: &[String]) -> bool {
        let dashboard_group = self.dashboard_group();
        if !self.spec.expose_via_dashboard {
            return false;
        }
        user_groups.iter().any(|g| g == &dashboard_group)
    }

    /// The virtual APIs enabled on this cluster, deduplicated.
    #[must_use]
    pub fn enabled_virtual_apis(&self) -> Vec<VirtualApiKind> {
        enabled_kinds(&self.spec.virtual_apis)
    }

    /// The group a user must belong to in order to proxy requests to this cluster.
    ///
    /// `None` means the cluster is not group-restricted: any successfully
    /// authenticated caller may reach it. An explicit `proxy_group` always
    /// wins; otherwise a dashboard-exposed cluster reuses its dashboard group so
    /// that listing a cluster and using it require the same membership.
    #[must_use]
    pub fn proxy_group(&self) -> Option<String> {
        if let Some(group) = &self.spec.proxy_group {
            return Some(group.clone());
        }
        if self.spec.expose_via_dashboard {
            return Some(self.dashboard_group());
        }
        None
    }

    /// Whether the proxy is restricted to a group at all.
    #[must_use]
    pub fn is_proxy_group_restricted(&self) -> bool {
        self.proxy_group().is_some()
    }

    /// Whether `user_groups` may proxy requests to this cluster.
    #[must_use]
    pub fn is_proxy_allowed(&self, user_groups: &[String]) -> bool {
        match self.proxy_group() {
            Some(proxy_group) => user_groups.iter().any(|group| group == &proxy_group),
            None => true,
        }
    }

    #[must_use]
    pub fn need_token_validation(&self) -> bool {
        if let Some(auth_config) = &self.spec.auth_config {
            !auth_config.disable_validation
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authentication_configuration::{OidcProvider, ValidateAgainst};
    use crate::security::{AllowedPathConfiguration, AllowedPathConfigurationEnum};

    fn oidc_provider(enabled: bool) -> OidcProvider {
        OidcProvider {
            enabled,
            issuer_url: "https://issuer.example.com".to_string(),
            client_id: "proxyauthk8s".to_string(),
            client_secret: Some("secret".to_string()),
            extra_scope: "groups".to_string(),
            audience: String::new(),
            accept_authorized_party: false,
            expose_oauth_authorization_server: false,
            config_from: None,
            claim_mappings: None,
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
            cert: CertSource::SystemRoots(true),
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
            upstream_auth: None,
        }
    }

    fn proxy(spec: ProxyKubeApiSpec) -> ProxyKubeApi {
        let mut proxy = ProxyKubeApi::new("local-sso", spec);
        proxy.metadata.namespace = Some("default".to_string());
        proxy
    }

    /// The redirect handler logs `debug!(proxy = ?proxy)` on every request:
    /// nothing secret may come out of a formatted `ProxyKubeApi`.
    #[test]
    fn debug_output_of_a_proxy_holds_no_secret() {
        let mut spec = spec();
        let mut auth_config = auth_config(ValidateAgainst::OidcProvider, true);
        auth_config.oidc_provider.client_secret = Some("oidc-s3cr3t".to_string());
        spec.auth_config = Some(auth_config);
        spec.client_cert = Some(certificate::ClientCertificate {
            cert: CertSource::Cert("cert-s3cr3t".to_string()),
            key: CertSource::Cert("key-s3cr3t".to_string()),
        });

        let printed = format!("{:?}", proxy(spec));
        for (field, secret) in [
            ("oidc", "oidc-s3cr3t"),
            ("cert", "cert-s3cr3t"),
            ("key", "key-s3cr3t"),
        ] {
            assert!(
                !printed.contains(secret),
                "the {field} secret leaked into Debug output"
            );
        }
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
    fn validate_rejects_well_known_discovery_without_an_enabled_provider() {
        let mut spec = spec();
        let mut config = auth_config(ValidateAgainst::Kubernetes, false);
        config.oidc_provider.expose_oauth_authorization_server = true;
        spec.auth_config = Some(config);
        let err = proxy(spec).validate().unwrap_err();
        assert!(
            err.contains("expose_oauth_authorization_server"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_accepts_well_known_discovery_with_an_enabled_provider() {
        let mut spec = spec();
        let mut config = auth_config(ValidateAgainst::Kubernetes, true);
        config.oidc_provider.expose_oauth_authorization_server = true;
        spec.auth_config = Some(config);
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
                    allowed_ports: None,
                    exec: None,
                    verbs: None,
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
            proxy(spec()).dashboard_group(),
            "dashboard-default-local-sso"
        );
    }

    #[test]
    fn dashboard_group_uses_the_configured_value() {
        let mut spec = spec();
        spec.dashboard_group = Some("platform-admins".to_string());
        assert_eq!(proxy(spec).dashboard_group(), "platform-admins");
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
            "cert": { "SystemRoots": true },
            "service": { "ExternalService": { "url": "https://cluster.example.com:6443" } }
        });
        let spec: ProxyKubeApiSpec = serde_json::from_value(json).expect("spec should deserialize");
        assert!(!spec.expose_via_dashboard);
    }

    #[test]
    fn a_plain_cluster_is_not_group_restricted() {
        let proxy = proxy(spec());
        assert_eq!(proxy.proxy_group(), None);
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
            proxy.proxy_group().as_deref(),
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

        assert_eq!(proxy.proxy_group().as_deref(), Some("operators"));
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

    fn trusted(mode: upstream_auth::UpstreamAuthMode) -> ProxyKubeApiSpec {
        let mut spec = spec();
        let mut config = auth_config(ValidateAgainst::OidcProvider, true);
        config.oidc_provider.claim_mappings = Some(
            serde_json::from_value(serde_json::json!({
                "username": { "claim": "sub", "prefix": "" }
            }))
            .expect("mapping"),
        );
        spec.auth_config = Some(config);
        spec.upstream_auth = Some(UpstreamAuth {
            mode,
            user_prefix: Some("proxyauth:".to_string()),
            group_prefix: Some("proxyauth:".to_string()),
            issuer_key: None,
        });
        spec
    }

    #[test]
    fn a_complete_front_proxy_spec_is_valid() {
        let proxy = proxy(trusted(upstream_auth::UpstreamAuthMode::FrontProxy));
        assert!(proxy.validate().is_ok(), "{:?}", proxy.validate());
        assert_eq!(
            proxy.upstream_mode(),
            upstream_auth::UpstreamAuthMode::FrontProxy
        );
    }

    #[test]
    fn an_absent_upstream_block_is_passthrough() {
        assert_eq!(
            proxy(spec()).upstream_mode(),
            upstream_auth::UpstreamAuthMode::Passthrough
        );
    }

    #[test]
    fn trusted_modes_refuse_what_would_make_the_asserted_identity_unsafe() {
        use upstream_auth::UpstreamAuthMode::{FrontProxy, Issuer};
        let cases: [(&str, Box<dyn Fn(&mut ProxyKubeApiSpec)>); 6] = [
            (
                "client_cert",
                Box::new(|s| {
                    s.client_cert = Some(certificate::ClientCertificate {
                        cert: CertSource::Cert("c".into()),
                        key: CertSource::Cert("k".into()),
                    })
                }),
            ),
            (
                "Kubernetes",
                Box::new(|s| {
                    s.auth_config.as_mut().unwrap().validate_against = ValidateAgainst::Kubernetes
                }),
            ),
            (
                "disable_validation",
                Box::new(|s| s.auth_config.as_mut().unwrap().disable_validation = true),
            ),
            (
                "claim_mappings",
                Box::new(|s| s.auth_config.as_mut().unwrap().oidc_provider.claim_mappings = None),
            ),
            (
                "user_prefix",
                Box::new(|s| {
                    s.upstream_auth.as_mut().unwrap().user_prefix = Some("system:".into())
                }),
            ),
            (
                "group_prefix",
                Box::new(|s| s.upstream_auth.as_mut().unwrap().group_prefix = None),
            ),
        ];
        for mode in [FrontProxy, Issuer] {
            for (needle, break_it) in &cases {
                let mut spec = trusted(mode);
                if mode == Issuer {
                    spec.upstream_auth.as_mut().unwrap().issuer_key =
                        Some(upstream_auth::IssuerKeySettings {
                            issuer_url: "https://proxy.example.com/issuer/default/local-sso".into(),
                            token_ttl_seconds: 300,
                            rotation_days: 30,
                        });
                }
                break_it(&mut spec);
                let err = proxy(spec).validate().unwrap_err();
                assert!(err.contains(needle), "{mode:?} {needle}: {err}");
            }
        }
    }

    #[test]
    fn an_extra_key_that_cannot_be_forwarded_is_refused_in_a_trusted_mode() {
        let mut spec = trusted(upstream_auth::UpstreamAuthMode::FrontProxy);
        spec.auth_config
            .as_mut()
            .unwrap()
            .oidc_provider
            .claim_mappings = Some(
            serde_json::from_value(serde_json::json!({
                "username": { "claim": "sub", "prefix": "" },
                "extra": [{ "key": "email", "value_expression": "claims.email" }]
            }))
            .expect("mapping"),
        );
        let err = proxy(spec.clone()).validate().unwrap_err();
        assert!(err.contains("email"), "{err}");
        // The same mapping stays valid in Passthrough: nothing is forwarded.
        spec.upstream_auth.as_mut().unwrap().mode = upstream_auth::UpstreamAuthMode::Passthrough;
        assert!(proxy(spec).validate().is_ok());
    }

    #[test]
    fn the_issuer_url_must_be_the_route_served_for_this_cluster() {
        let mut spec = trusted(upstream_auth::UpstreamAuthMode::Issuer);
        spec.upstream_auth.as_mut().unwrap().issuer_key = Some(upstream_auth::IssuerKeySettings {
            issuer_url: "https://proxy.example.com/issuer/other/cluster".into(),
            token_ttl_seconds: 300,
            rotation_days: 30,
        });
        let err = proxy(spec).validate().unwrap_err();
        assert!(err.contains("/issuer/default/local-sso"), "{err}");
    }

    /// The regression this guards: the redirect workers record the whole
    /// resource as a span field (`fields(proxy = ?ctx.proxy)`) on every proxied
    /// request, so a derived `Debug` on any nested type puts that type's secrets
    /// in the trace backend. Asserted here on the full `ProxyKubeApi` rather
    /// than only on `OidcProvider`, because that is the shape actually logged.
    #[test]
    fn debug_of_the_whole_resource_never_carries_the_oidc_client_secret() {
        let mut spec = spec();
        let mut config = auth_config(ValidateAgainst::OidcProvider, true);
        config.oidc_provider.client_secret = Some("unmistakable-client-secret".to_string());
        spec.auth_config = Some(config);

        let rendered = format!("{:?}", proxy(spec));
        assert!(
            !rendered.contains("unmistakable-client-secret"),
            "client_secret leaked into ProxyKubeApi Debug output: {rendered}"
        );
        assert!(rendered.contains("***REDACTED***"), "{rendered}");
    }
}
