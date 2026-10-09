use crate::default::{default_empty_array, default_enabled};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod allowed_crd_configuration;
mod allowed_path_configuration;
mod allowed_path_configuration_enum;
mod fail2login_equal_ban_configuration;
mod namespaced_access_configuration;
mod namespaced_access_rule_kind;
pub mod path_matcher;
mod per_user_group_rate_limiting_configuration;
mod port_range;
mod rate_limiting_configuration;

pub use allowed_crd_configuration::AllowedCrdConfiguration;
pub use allowed_path_configuration::AllowedPathConfiguration;
pub use allowed_path_configuration_enum::AllowedPathConfigurationEnum;
pub use fail2login_equal_ban_configuration::Fail2LoginEqualBanConfiguration;
pub use namespaced_access_configuration::NamespacedAccessConfiguration;
pub use namespaced_access_rule_kind::NamespacedAccessRuleKind;
pub use per_user_group_rate_limiting_configuration::PerUserGroupRateLimitingConfiguration;
pub use port_range::{PortPolicy, PortSpec};
pub use rate_limiting_configuration::RateLimitingConfiguration;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SecurityConfiguration {
    /// Whether the token is validated beforehand
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Configuration for banning callers after multiple failed authentications
    #[serde(default)]
    pub fail2login_equal_ban: Fail2LoginEqualBanConfiguration,
    /// Global rate limiting configuration
    #[serde(default)]
    pub rate_limiting: RateLimitingConfiguration,
    /// Per group rate limiting configuration
    /// This takes precedence over the global rate limiting configuration
    #[serde(default = "default_empty_array::<PerUserGroupRateLimitingConfiguration>")]
    pub per_user_group_rate_limiting: Vec<PerUserGroupRateLimitingConfiguration>,
    /// Allowed resources, limit the access to the proxy to only these resources, if empty all resources are allowed
    ///
    /// The bound keeps the estimated cost of the per-item CEL rules within the
    /// budget the apiserver allows for a whole CRD schema.
    #[serde(default = "default_empty_array::<AllowedPathConfigurationEnum>")]
    #[schemars(length(max = 128))]
    pub allowed_resources: Vec<AllowedPathConfigurationEnum>,

    /// Deprecated misspelling of `allowed_resources`, merged with it
    ///
    /// Kept as a real field rather than a serde alias on purpose: the apiserver
    /// prunes fields absent from the schema, so an alias alone would silently
    /// drop the security configuration of every existing resource. Remove it in
    /// the next major, once resources have been migrated.
    #[serde(default = "default_empty_array::<AllowedPathConfigurationEnum>")]
    #[schemars(length(max = 128))]
    pub allowed_ressources: Vec<AllowedPathConfigurationEnum>,
}

impl Default for SecurityConfiguration {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            fail2login_equal_ban: Fail2LoginEqualBanConfiguration::default(),
            rate_limiting: RateLimitingConfiguration::default(),
            per_user_group_rate_limiting: default_empty_array(),
            allowed_resources: default_empty_array(),
            allowed_ressources: default_empty_array(),
        }
    }
}

impl SecurityConfiguration {
    /// Every configured rule, whichever spelling it was written under.
    pub fn all_allowed_resources(&self) -> impl Iterator<Item = &AllowedPathConfigurationEnum> {
        self.allowed_resources
            .iter()
            .chain(self.allowed_ressources.iter())
    }

    pub fn validate(&self) -> Result<(), String> {
        for allowed_resource in self.all_allowed_resources() {
            allowed_resource.validate()?;
        }
        Ok(())
    }

    /// Requests per minute allowed for a caller in `groups`.
    ///
    /// `None` means no limit at all. A group entry always wins over the global
    /// setting, and when a caller belongs to several configured groups the most
    /// permissive one applies — being in an extra group must never make a user
    /// more restricted. A configured `0` means unlimited.
    #[must_use]
    pub fn requests_per_minute(&self, groups: &[String]) -> Option<u32> {
        if !self.enabled {
            return None;
        }

        let group_limits: Vec<u32> = self
            .per_user_group_rate_limiting
            .iter()
            .filter(|entry| groups.iter().any(|group| group == &entry.group))
            .map(|entry| entry.max_requests_per_minute)
            .collect();

        if !group_limits.is_empty() {
            // A configured 0 is "unlimited", so it wins over any other value.
            if group_limits.contains(&0) {
                return None;
            }
            return group_limits.into_iter().max();
        }

        if !self.rate_limiting.enabled || self.rate_limiting.max_requests_per_minute == 0 {
            return None;
        }
        Some(self.rate_limiting.max_requests_per_minute)
    }

    /// Whether failed authentications should be counted and banned.
    #[must_use]
    pub fn fail2login_enabled(&self) -> bool {
        self.enabled && self.fail2login_equal_ban.enabled
    }

    /// Ban duration after `failures` failed authentications.
    ///
    /// `None` means the caller is not banned yet. `Some(0)` is a permanent ban.
    #[must_use]
    pub fn ban_duration_for(&self, failures: u32) -> Option<u32> {
        if !self.fail2login_enabled() {
            return None;
        }
        let config = &self.fail2login_equal_ban;
        if failures < config.max_failed_logins {
            return None;
        }
        if config.ban_duration == 0 {
            return Some(0);
        }
        if !config.exponential_backoff {
            return Some(config.ban_duration);
        }

        // Each failure past the threshold doubles the ban, capped so a long-lived
        // counter cannot produce an effectively permanent one by accident.
        let extra = (failures - config.max_failed_logins).min(16);
        Some(config.ban_duration.saturating_mul(1u32 << extra))
    }

    /// Check whether an upstream request path may be forwarded.
    ///
    /// `path` is the path sent to the Kubernetes API (the
    /// `/clusters/{ns}/{cluster}` prefix already stripped) without its query
    /// string. An empty allowed resource list, or a disabled security
    /// configuration, allows everything — that is the documented behaviour.
    #[must_use]
    pub fn is_path_allowed(&self, path: &str, username: &str, groups: &[String]) -> bool {
        let mut rules = self.all_allowed_resources().peekable();
        if !self.enabled || rules.peek().is_none() {
            return true;
        }
        rules.any(|allowed_resource| allowed_resource.matches(path, username, groups))
    }

    /// Ports a port-forward to `path` may open.
    ///
    /// The ports of every matching rule are combined; a matching rule without
    /// `allowed_ports` (and any `Crd` rule) lifts the restriction. With no rule
    /// at all, or a disabled configuration, nothing is restricted, as for
    /// [`Self::is_path_allowed`]. With rules but none matching, no port is
    /// allowed (the path itself is refused anyway).
    #[must_use]
    pub fn port_forward_policy(&self, path: &str, username: &str, groups: &[String]) -> PortPolicy {
        let mut rules = self.all_allowed_resources().peekable();
        if !self.enabled || rules.peek().is_none() {
            return PortPolicy::Any;
        }

        let mut allowed = Vec::new();
        for rule in rules.filter(|rule| rule.matches(path, username, groups)) {
            match rule.port_policy() {
                PortPolicy::Any => return PortPolicy::Any,
                PortPolicy::Only(ranges) => allowed.extend(ranges),
            }
        }
        PortPolicy::Only(allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path_rule(path: &str, parametised: bool) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Path(AllowedPathConfiguration {
            path: path.to_string(),
            parametised,
            allowed_ports: None,
        })
    }

    fn port_forward_rule(path: &str, ports: Option<&[&str]>) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Path(AllowedPathConfiguration {
            path: path.to_string(),
            parametised: true,
            allowed_ports: ports.map(|ports| {
                ports
                    .iter()
                    .map(|port| PortSpec((*port).to_string()))
                    .collect()
            }),
        })
    }

    const PORT_FORWARD: &str = "/api/v1/namespaces/dev/pods/web/portforward";

    #[test]
    fn port_forward_is_unrestricted_without_rules() {
        let config = SecurityConfiguration::default();
        assert_eq!(
            config.port_forward_policy(PORT_FORWARD, "alice", &[]),
            PortPolicy::Any
        );
    }

    #[test]
    fn port_forward_ports_of_matching_rules_are_combined() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/dev/pods/*/portforward", Some(&["8080"])),
                port_forward_rule(
                    "/api/v1/namespaces/*/pods/web/portforward",
                    Some(&["9000-9001"]),
                ),
                port_forward_rule("/api/v1/namespaces/prod/pods/*/portforward", Some(&["22"])),
            ],
            ..SecurityConfiguration::default()
        };

        let policy = config.port_forward_policy(PORT_FORWARD, "alice", &[]);
        assert!(policy.allows(8080));
        assert!(policy.allows(9001));
        assert!(!policy.allows(22));
    }

    #[test]
    fn a_matching_rule_without_ports_lifts_the_restriction() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/dev/pods/*/portforward", Some(&["8080"])),
                port_forward_rule("/api/v1/namespaces/dev/**", None),
            ],
            ..SecurityConfiguration::default()
        };
        assert_eq!(
            config.port_forward_policy(PORT_FORWARD, "alice", &[]),
            PortPolicy::Any
        );
    }

    #[test]
    fn invalid_and_empty_port_lists_allow_nothing() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule(
                    "/api/v1/namespaces/dev/pods/*/portforward",
                    Some(&["0", "70000"]),
                ),
                port_forward_rule("/api/v1/namespaces/dev/pods/web/portforward", Some(&[])),
            ],
            ..SecurityConfiguration::default()
        };
        assert_eq!(
            config.port_forward_policy(PORT_FORWARD, "alice", &[]),
            PortPolicy::Only(Vec::new())
        );
        assert!(config.validate().is_err());
    }

    /// `pods/web%2Fportforward` reaches the apiserver as a port-forward but is
    /// not recognised as one by the proxy, so it would get no port policy: the
    /// path itself must be refused instead, even by a rule its raw form fits.
    #[test]
    fn an_encoded_slash_cannot_hide_a_port_forward() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/dev/pods/*/portforward", Some(&["8080"])),
                path_rule("/api/v1/namespaces/dev/pods/*", true),
            ],
            ..SecurityConfiguration::default()
        };
        for path in [
            "/api/v1/namespaces/dev/pods/web%2Fportforward",
            "/api/v1/namespaces/dev/pods/web%2fportforward",
        ] {
            assert!(!config.is_path_allowed(path, "alice", &[]), "{path}");
        }
    }

    #[test]
    fn empty_allow_list_allows_everything() {
        let config = SecurityConfiguration::default();
        assert!(config.is_path_allowed("/api/v1/namespaces/kube-system/secrets", "alice", &[]));
    }

    /// A resource written before the field was spelled correctly must keep
    /// working: the deprecated field is still read, and still enforced.
    #[test]
    fn the_deprecated_spelling_is_still_enforced() {
        let config: SecurityConfiguration = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "allowed_ressources": [{ "Path": { "path": "/api/v1/pods" } }],
        }))
        .expect("a legacy configuration should deserialize");

        assert!(config.allowed_resources.is_empty());
        assert_eq!(config.allowed_ressources.len(), 1);
        assert!(config.is_path_allowed("/api/v1/pods", "alice", &[]));
        assert!(!config.is_path_allowed("/api/v1/secrets", "alice", &[]));
    }

    #[test]
    fn both_spellings_are_merged() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/pods", false)],
            allowed_ressources: vec![path_rule("/api/v1/configmaps", false)],
            ..SecurityConfiguration::default()
        };

        assert_eq!(config.all_allowed_resources().count(), 2);
        assert!(config.is_path_allowed("/api/v1/pods", "alice", &[]));
        assert!(config.is_path_allowed("/api/v1/configmaps", "alice", &[]));
        assert!(!config.is_path_allowed("/api/v1/secrets", "alice", &[]));
    }

    #[test]
    fn the_deprecated_spelling_is_validated_too() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_ressources: vec![path_rule("/api/v1/namespaces/{{tenant}}/pods", true)],
            ..SecurityConfiguration::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn disabled_configuration_allows_everything() {
        let config = SecurityConfiguration {
            enabled: false,
            allowed_resources: vec![path_rule("/api/v1/pods", false)],
            ..SecurityConfiguration::default()
        };
        assert!(config.is_path_allowed("/api/v1/namespaces/kube-system/secrets", "alice", &[]));
    }

    #[test]
    fn non_matching_path_is_rejected() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/namespaces/{{group}}/pods/**", true)],
            ..SecurityConfiguration::default()
        };
        let groups = vec!["dev-alice".to_string()];

        assert!(config.is_path_allowed("/api/v1/namespaces/dev-alice/pods", "alice", &groups));
        assert!(config.is_path_allowed(
            "/api/v1/namespaces/dev-alice/pods/mypod/log",
            "alice",
            &groups
        ));
        assert!(!config.is_path_allowed(
            "/api/v1/namespaces/kube-system/secrets",
            "alice",
            &groups
        ));
    }

    fn crd_rule(namespace: NamespacedAccessConfiguration) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Crd(AllowedCrdConfiguration {
            group: "example.com".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: Some("widgets".to_string()),
            namespace,
            namespaced: true,
        })
    }

    #[test]
    fn a_crd_rule_enforces_group_version_kind_and_namespace() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![crd_rule(NamespacedAccessConfiguration {
                enabled: true,
                rule_kind: NamespacedAccessRuleKind::AllowedNamespaces(vec!["dev".to_string()]),
            })],
            ..SecurityConfiguration::default()
        };

        assert!(config.is_path_allowed(
            "/apis/example.com/v1/namespaces/dev/widgets",
            "alice",
            &[]
        ));
        // Right resource, wrong (denied) namespace.
        assert!(!config.is_path_allowed(
            "/apis/example.com/v1/namespaces/kube-system/widgets",
            "alice",
            &[]
        ));
        // Different resource entirely.
        assert!(!config.is_path_allowed("/api/v1/namespaces/dev/secrets", "alice", &[]));
    }

    #[test]
    fn crd_and_path_rules_coexist_in_the_allow_list() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                path_rule("/api/v1/namespaces/dev/pods", false),
                crd_rule(NamespacedAccessConfiguration {
                    enabled: false,
                    rule_kind: NamespacedAccessRuleKind::AllowedNamespaces(vec![]),
                }),
            ],
            ..SecurityConfiguration::default()
        };
        assert!(config.is_path_allowed("/api/v1/namespaces/dev/pods", "alice", &[]));
        assert!(config.is_path_allowed(
            "/apis/example.com/v1/namespaces/anything/widgets",
            "alice",
            &[]
        ));
        assert!(!config.is_path_allowed("/api/v1/namespaces/dev/secrets", "alice", &[]));
    }

    #[test]
    fn any_matching_rule_allows_the_path() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                path_rule("/api/v1/namespaces/dev/pods", false),
                path_rule("/apis/apps/v1/namespaces/*/deployments", true),
            ],
            ..SecurityConfiguration::default()
        };
        assert!(config.is_path_allowed("/api/v1/namespaces/dev/pods", "alice", &[]));
        assert!(config.is_path_allowed(
            "/apis/apps/v1/namespaces/staging/deployments",
            "alice",
            &[]
        ));
        assert!(!config.is_path_allowed("/api/v1/namespaces/dev/secrets", "alice", &[]));
    }

    fn with_rate_limiting(global: Option<u32>, groups: Vec<(&str, u32)>) -> SecurityConfiguration {
        SecurityConfiguration {
            rate_limiting: RateLimitingConfiguration {
                enabled: global.is_some(),
                max_requests_per_minute: global.unwrap_or_default(),
            },
            per_user_group_rate_limiting: groups
                .into_iter()
                .map(|(group, max)| PerUserGroupRateLimitingConfiguration {
                    group: group.to_string(),
                    max_requests_per_minute: max,
                })
                .collect(),
            ..SecurityConfiguration::default()
        }
    }

    #[test]
    fn rate_limiting_is_off_by_default() {
        assert_eq!(
            SecurityConfiguration::default().requests_per_minute(&[]),
            None
        );
    }

    #[test]
    fn the_global_limit_applies_without_a_group_entry() {
        let config = with_rate_limiting(Some(60), vec![]);
        assert_eq!(config.requests_per_minute(&[]), Some(60));
        assert_eq!(
            config.requests_per_minute(&["unrelated".to_string()]),
            Some(60)
        );
    }

    #[test]
    fn a_group_limit_overrides_the_global_one() {
        let config = with_rate_limiting(Some(60), vec![("power-users", 600)]);
        assert_eq!(
            config.requests_per_minute(&["power-users".to_string()]),
            Some(600)
        );
        assert_eq!(
            config.requests_per_minute(&["others".to_string()]),
            Some(60)
        );
    }

    #[test]
    fn the_most_permissive_group_wins() {
        let config = with_rate_limiting(Some(60), vec![("a", 100), ("b", 500)]);
        assert_eq!(
            config.requests_per_minute(&["a".to_string(), "b".to_string()]),
            Some(500)
        );
    }

    #[test]
    fn a_zero_group_limit_means_unlimited() {
        let config = with_rate_limiting(Some(60), vec![("admins", 0), ("devs", 100)]);
        assert_eq!(config.requests_per_minute(&["admins".to_string()]), None);
        assert_eq!(
            config.requests_per_minute(&["admins".to_string(), "devs".to_string()]),
            None
        );
    }

    #[test]
    fn a_disabled_security_config_never_rate_limits() {
        let mut config = with_rate_limiting(Some(60), vec![]);
        config.enabled = false;
        assert_eq!(config.requests_per_minute(&[]), None);
    }

    fn with_fail2login(
        max_failed_logins: u32,
        ban_duration: u32,
        exponential_backoff: bool,
    ) -> SecurityConfiguration {
        SecurityConfiguration {
            fail2login_equal_ban: Fail2LoginEqualBanConfiguration {
                enabled: true,
                max_failed_logins,
                ban_duration,
                exponential_backoff,
            },
            ..SecurityConfiguration::default()
        }
    }

    #[test]
    fn fail2login_is_off_by_default() {
        let config = SecurityConfiguration::default();
        assert!(!config.fail2login_enabled());
        assert_eq!(config.ban_duration_for(100), None);
    }

    #[test]
    fn no_ban_below_the_threshold() {
        let config = with_fail2login(5, 300, false);
        assert_eq!(config.ban_duration_for(4), None);
        assert_eq!(config.ban_duration_for(5), Some(300));
        assert_eq!(config.ban_duration_for(9), Some(300));
    }

    #[test]
    fn exponential_backoff_doubles_each_extra_failure() {
        let config = with_fail2login(3, 60, true);
        assert_eq!(config.ban_duration_for(2), None);
        assert_eq!(config.ban_duration_for(3), Some(60));
        assert_eq!(config.ban_duration_for(4), Some(120));
        assert_eq!(config.ban_duration_for(5), Some(240));
        // Capped rather than overflowing into an accidental permanent ban.
        assert!(config.ban_duration_for(1000).unwrap() > 0);
    }

    #[test]
    fn a_zero_duration_is_a_permanent_ban() {
        let config = with_fail2login(3, 0, true);
        assert_eq!(config.ban_duration_for(3), Some(0));
    }

    #[test]
    fn validate_rejects_invalid_parameters() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/namespaces/{{tenant}}/pods", true)],
            ..SecurityConfiguration::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_configuration() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/namespaces/{{username}}/pods", true)],
            ..SecurityConfiguration::default()
        };
        assert!(config.validate().is_ok());
    }
}
