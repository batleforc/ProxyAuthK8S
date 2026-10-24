//! `spec.service.Agent` (RFC 0012): the cluster is reached through an agent
//! that dials out to the proxy, never by a connection from the proxy.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::certificate::CertSource;

/// SAN checked on the remote apiserver's certificate when nothing else is set.
pub const DEFAULT_SERVER_NAME: &str = "kubernetes.default.svc";
/// `aud` an agent's projected ServiceAccount token must carry by default.
pub const DEFAULT_AUDIENCE: &str = "proxyauth";
/// Prefix every agent `subject` starts with: only a ServiceAccount is an agent.
pub const SERVICE_ACCOUNT_PREFIX: &str = "system:serviceaccount:";
/// Bound on an inline or fetched JWKS (RFC 0012 §4.3).
pub const MAX_JWKS_BYTES: usize = 64 * 1024;
/// A new value on a `ProxyKubeApi` asks the controller for a new
/// `OneShotToken` (RFC 0012 §6.4).
pub const REGENERATE_BOOTSTRAP_ANNOTATION: &str = "proxyauth.weebo.si.rs/regenerate-bootstrap";

/// The Secret holding the `OneShotToken` of `ProxyKubeApi` `name` (key
/// `token`), next to it.
#[must_use]
pub fn bootstrap_secret_name(name: &str) -> String {
    format!("{name}-agent-bootstrap")
}

fn default_server_name() -> String {
    DEFAULT_SERVER_NAME.to_string()
}

fn default_audience() -> String {
    DEFAULT_AUDIENCE.to_string()
}

/// How the proxy reaches a cluster whose apiserver accepts no inbound
/// connection: an agent on that cluster opens a tunnel to the proxy, and the
/// proxy's TLS to the apiserver runs inside it, end to end.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "self.server_name.matches('^[a-z0-9]([-a-z0-9.]*[a-z0-9])?$') || isIP(self.server_name)",
        "message": "server_name must be a DNS name or an IP address",
    }),
    serde_json::json!({
        "rule": "self.epoch >= oldSelf.epoch",
        "message": "epoch must not decrease: a lower epoch would make revoked agent certificates valid again",
    }),
]))]
pub struct AgentService {
    /// Name checked against the remote apiserver's certificate
    /// Default: kubernetes.default.svc
    #[serde(default = "default_server_name")]
    #[schemars(length(min = 1, max = 253))]
    pub server_name: String,
    /// Raise it to revoke every agent certificate issued so far: open tunnels
    /// close within 30 s and the agent has to enrol again
    /// Default: 0
    #[serde(default)]
    pub epoch: u64,
    /// How an agent proves it serves this cluster before it gets a certificate
    pub bootstrap: AgentBootstrap,
    /// Overrides of the gateway's tunnel bounds, for this cluster only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<AgentLimits>,
}

/// Per-cluster tunnel bounds (RFC 0012, decision 27).
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq, Eq)]
pub struct AgentLimits {
    /// Streams one tunnel carries at once, for a cluster with many concurrent
    /// clients (an operator driving it through the proxy). Capped by the
    /// gateway's maxStreamsPerTunnel, with a warning on the CR
    /// Default: the gateway's streamsPerTunnel
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub streams_per_tunnel: Option<u32>,
}

/// The proof an agent presents at enrolment. One variant.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum AgentBootstrap {
    /// A projected ServiceAccount token of the remote cluster, checked against
    /// that cluster's public keys: nothing secret goes into git
    ServiceAccountToken(ServiceAccountTokenBootstrap),
    /// A single-use token the controller writes to the Secret
    /// `<name>-agent-bootstrap` next to this resource, valid one hour; carry
    /// it to the remote cluster (External Secrets, Vault)
    OneShotToken(OneShotTokenBootstrap),
    /// No enrolment: the agent's certificate comes from an issuer of yours
    /// (cert-manager, Vault PKI), with the URI SAN proxyauth://<namespace>/<name>
    /// of this resource, and chains to the CA named here
    ExternalCA(ExternalCaBootstrap),
}

fn default_max_cert_lifetime_hours() -> u32 {
    DEFAULT_MAX_CERT_LIFETIME_HOURS
}

/// Longest external agent certificate accepted by default: `epoch` cannot
/// revoke a certificate the proxy did not issue (decision 29).
pub const DEFAULT_MAX_CERT_LIFETIME_HOURS: u32 = 7 * 24;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "has(self.ca) != has(self.ca_ref)",
        "message": "set exactly one of ca and ca_ref",
    }),
]))]
pub struct ExternalCaBootstrap {
    /// The issuing CA, PEM, inline (public)
    #[schemars(length(max = 65536))]
    pub ca: Option<String>,
    /// A ConfigMap or Secret holding the issuing CA, PEM, re-read at each
    /// connection (same namespace policy as `cert`)
    pub ca_ref: Option<JwksSource>,
    /// A longer certificate is refused, and a tunnel closes when its
    /// certificate expires: raising `epoch` only closes the open tunnels,
    /// revoke at the issuer and keep certificates short
    /// Default: 168 (7 days)
    #[serde(default = "default_max_cert_lifetime_hours")]
    #[schemars(range(min = 1, max = 8760))]
    pub max_cert_lifetime_hours: u32,
}

impl ExternalCaBootstrap {
    fn validate(&self) -> Result<(), String> {
        if self.ca.is_some() == self.ca_ref.is_some() {
            return Err("set exactly one of ca and ca_ref".to_string());
        }
        if self.ca.as_ref().is_some_and(|ca| ca.len() > MAX_JWKS_BYTES) {
            return Err(format!("ca is larger than {MAX_JWKS_BYTES} bytes"));
        }
        if !(1..=8760).contains(&self.max_cert_lifetime_hours) {
            return Err("max_cert_lifetime_hours must be between 1 and 8760".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq, Eq)]
pub struct OneShotTokenBootstrap {
    /// Write a new token as soon as the previous one is used or expired;
    /// otherwise, annotate this resource with
    /// proxyauth.weebo.si.rs/regenerate-bootstrap (any new value) for one
    /// Default: false
    #[serde(default)]
    pub auto_rotate: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "[has(self.jwks), has(self.jwks_url), has(self.jwks_from)].filter(x, x).size() == 1",
        "message": "set exactly one of jwks, jwks_url and jwks_from",
    }),
    serde_json::json!({
        "rule": "!has(self.jwks_url) || self.jwks_url.startsWith('https://')",
        "message": "jwks_url must be an https:// URL",
    }),
    serde_json::json!({
        "rule": "self.subject.startsWith('system:serviceaccount:')",
        "message": "subject must be a ServiceAccount (system:serviceaccount:<namespace>:<name>)",
    }),
]))]
pub struct ServiceAccountTokenBootstrap {
    /// `sub` of the agent's token: `system:serviceaccount:<namespace>:<name>`
    #[schemars(length(max = 316))]
    pub subject: String,
    /// `aud` of the agent's token
    /// Default: proxyauth
    #[serde(default = "default_audience")]
    #[schemars(length(min = 1, max = 253))]
    pub audience: String,
    /// The remote cluster's JWKS, inline (`kubectl get --raw /openid/v1/jwks`)
    /// Frozen: a key rotation needs an edit
    #[schemars(length(max = 65536))]
    pub jwks: Option<String>,
    /// Where a public issuer serves its JWKS (EKS, GKE, AKS)
    #[schemars(length(max = 2048))]
    pub jwks_url: Option<String>,
    /// A ConfigMap or Secret holding the JWKS, re-read at enrolment: for an
    /// issuer only reachable inside the remote cluster (kubeadm, k3s)
    pub jwks_from: Option<JwksSource>,
}

/// Where `jwks_from` reads the JWKS. Public keys: a ConfigMap is enough.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum JwksSource {
    ConfigMap {
        name: String,
        key: String,
        /// If not set, the resource namespace (same policy as `cert`)
        namespace: Option<String>,
    },
    Secret {
        name: String,
        key: String,
        /// If not set, the resource namespace (same policy as `cert`)
        namespace: Option<String>,
    },
}

impl JwksSource {
    /// The same reference as a `CertSource`, so the read goes through the one
    /// code path that applies the cross-namespace policy.
    #[must_use]
    pub fn as_cert_source(&self) -> CertSource {
        match self.clone() {
            Self::ConfigMap {
                name,
                key,
                namespace,
            } => CertSource::ConfigMap {
                name,
                key,
                namespace,
            },
            Self::Secret {
                name,
                key,
                namespace,
            } => CertSource::Secret {
                name,
                key,
                namespace,
            },
        }
    }
}

impl AgentService {
    /// Streams per tunnel for this cluster: its override, else `default`,
    /// never above `max`. `true` when the override was capped.
    #[must_use]
    pub fn streams_per_tunnel(&self, default: usize, max: usize) -> (usize, bool) {
        match self
            .limits
            .as_ref()
            .and_then(|limits| limits.streams_per_tunnel)
        {
            Some(wanted) => {
                let wanted = usize::try_from(wanted).unwrap_or(usize::MAX);
                (wanted.min(max), wanted > max)
            }
            None => (default.min(max), false),
        }
    }

    /// What the admission rules refuse, for a spec that skipped admission.
    pub fn validate(&self, cert: &CertSource) -> Result<(), String> {
        if matches!(cert, CertSource::SystemRoots(_) | CertSource::Insecure(_)) {
            return Err(
                "service Agent needs the remote apiserver's CA in cert: SystemRoots would accept any public certificate an agent presents"
                    .to_string(),
            );
        }
        match &self.bootstrap {
            AgentBootstrap::ServiceAccountToken(sa) => sa.validate(),
            AgentBootstrap::OneShotToken(_) => Ok(()),
            AgentBootstrap::ExternalCA(external) => external.validate(),
        }
    }
}

impl ServiceAccountTokenBootstrap {
    fn validate(&self) -> Result<(), String> {
        let sources = [
            self.jwks.is_some(),
            self.jwks_url.is_some(),
            self.jwks_from.is_some(),
        ];
        if sources.iter().filter(|set| **set).count() != 1 {
            return Err("set exactly one of jwks, jwks_url and jwks_from".to_string());
        }
        if self
            .jwks_url
            .as_deref()
            .is_some_and(|url| !url.starts_with("https://"))
        {
            return Err("jwks_url must be an https:// URL".to_string());
        }
        if !self.subject.starts_with(SERVICE_ACCOUNT_PREFIX) {
            return Err(format!(
                "subject must be a ServiceAccount ({SERVICE_ACCOUNT_PREFIX}<namespace>:<name>)"
            ));
        }
        if self
            .jwks
            .as_ref()
            .is_some_and(|jwks| jwks.len() > MAX_JWKS_BYTES)
        {
            return Err(format!("jwks is larger than {MAX_JWKS_BYTES} bytes"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_of_streams_per_tunnel_is_capped_by_the_gateway() {
        let mut service = agent(bootstrap());
        assert_eq!(service.streams_per_tunnel(128, 512), (128, false));
        service.limits = Some(AgentLimits {
            streams_per_tunnel: Some(384),
        });
        assert_eq!(service.streams_per_tunnel(128, 512), (384, false));
        service.limits = Some(AgentLimits {
            streams_per_tunnel: Some(4096),
        });
        assert_eq!(service.streams_per_tunnel(128, 512), (512, true));
    }

    fn bootstrap() -> ServiceAccountTokenBootstrap {
        ServiceAccountTokenBootstrap {
            subject: "system:serviceaccount:proxyauth-agent:proxyauth-agent".into(),
            audience: DEFAULT_AUDIENCE.into(),
            jwks: None,
            jwks_url: None,
            jwks_from: Some(JwksSource::ConfigMap {
                name: "prod-sa-jwks".into(),
                key: "jwks.json".into(),
                namespace: None,
            }),
        }
    }

    fn agent(sa: ServiceAccountTokenBootstrap) -> AgentService {
        AgentService {
            server_name: DEFAULT_SERVER_NAME.into(),
            epoch: 0,
            bootstrap: AgentBootstrap::ServiceAccountToken(sa),
            limits: None,
        }
    }

    fn ca() -> CertSource {
        CertSource::ConfigMap {
            name: "prod-ca".into(),
            key: "ca.crt".into(),
            namespace: None,
        }
    }

    #[test]
    fn a_single_jwks_source_with_a_private_ca_is_valid() {
        assert_eq!(agent(bootstrap()).validate(&ca()), Ok(()));
    }

    #[test]
    fn system_roots_are_refused_for_an_agent() {
        // T4: the CA never comes from the agent, nor from the public roots.
        assert!(
            agent(bootstrap())
                .validate(&CertSource::SystemRoots(true))
                .is_err()
        );
        assert!(
            agent(bootstrap())
                .validate(&CertSource::Insecure(true))
                .is_err()
        );
    }

    #[test]
    fn exactly_one_jwks_source_is_required() {
        let mut none = bootstrap();
        none.jwks_from = None;
        assert!(agent(none).validate(&ca()).is_err());

        let mut two = bootstrap();
        two.jwks = Some(r#"{"keys":[]}"#.into());
        assert!(agent(two).validate(&ca()).is_err());
    }

    #[test]
    fn a_plain_http_jwks_url_is_refused() {
        let mut sa = bootstrap();
        sa.jwks_from = None;
        sa.jwks_url = Some("http://issuer.example.com/keys".into());
        assert!(agent(sa.clone()).validate(&ca()).is_err());
        sa.jwks_url = Some("https://issuer.example.com/keys".into());
        assert_eq!(agent(sa).validate(&ca()), Ok(()));
    }

    #[test]
    fn only_a_service_account_can_be_the_subject() {
        let mut sa = bootstrap();
        sa.subject = "alice".into();
        assert!(agent(sa).validate(&ca()).is_err());
    }

    #[test]
    fn an_oversized_inline_jwks_is_refused() {
        let mut sa = bootstrap();
        sa.jwks_from = None;
        sa.jwks = Some("x".repeat(MAX_JWKS_BYTES + 1));
        assert!(agent(sa).validate(&ca()).is_err());
    }

    #[test]
    fn defaults_fill_the_server_name_the_epoch_and_the_audience() {
        let agent: AgentService = serde_json::from_value(serde_json::json!({
            "bootstrap": { "ServiceAccountToken": {
                "subject": "system:serviceaccount:a:b",
                "jwks_url": "https://issuer.example.com/keys",
            }},
        }))
        .expect("a minimal agent service should deserialize");
        assert_eq!(agent.server_name, DEFAULT_SERVER_NAME);
        assert_eq!(agent.epoch, 0);
        assert!(matches!(
            &agent.bootstrap,
            AgentBootstrap::ServiceAccountToken(sa) if sa.audience == DEFAULT_AUDIENCE
        ));
    }

    #[test]
    fn an_external_ca_needs_exactly_one_ca_and_a_bounded_lifetime() {
        let agent: AgentService = serde_json::from_value(serde_json::json!({
            "bootstrap": { "ExternalCA": { "ca_ref": { "ConfigMap": { "name": "vault-ca", "key": "ca.crt" } } } },
        }))
        .expect("an external CA agent service should deserialize");
        assert_eq!(agent.validate(&ca()), Ok(()));
        assert!(matches!(
            &agent.bootstrap,
            AgentBootstrap::ExternalCA(external)
                if external.max_cert_lifetime_hours == DEFAULT_MAX_CERT_LIFETIME_HOURS
        ));
        let mut external = ExternalCaBootstrap {
            ca: None,
            ca_ref: Some(JwksSource::ConfigMap {
                name: "vault-ca".into(),
                key: "ca.crt".into(),
                namespace: None,
            }),
            max_cert_lifetime_hours: DEFAULT_MAX_CERT_LIFETIME_HOURS,
        };
        external.ca = Some("-----BEGIN CERTIFICATE-----".into());
        assert!(external.validate().is_err(), "both");
        external.ca_ref = None;
        assert_eq!(external.validate(), Ok(()));
        external.ca = None;
        assert!(external.validate().is_err(), "neither");
        external.ca = Some("x".into());
        external.max_cert_lifetime_hours = 0;
        assert!(external.validate().is_err(), "zero lifetime");
    }

    #[test]
    fn a_one_shot_token_needs_no_more_than_its_name() {
        let agent: AgentService = serde_json::from_value(serde_json::json!({
            "bootstrap": { "OneShotToken": {} },
        }))
        .expect("a one-shot agent service should deserialize");
        assert!(matches!(
            &agent.bootstrap,
            AgentBootstrap::OneShotToken(OneShotTokenBootstrap { auto_rotate: false })
        ));
        assert_eq!(agent.validate(&ca()), Ok(()));
    }
}
