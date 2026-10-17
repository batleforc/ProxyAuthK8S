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
pub mod request_info;
mod verb;

pub use allowed_crd_configuration::AllowedCrdConfiguration;
pub use allowed_path_configuration::AllowedPathConfiguration;
pub use allowed_path_configuration_enum::AllowedPathConfigurationEnum;
pub use fail2login_equal_ban_configuration::Fail2LoginEqualBanConfiguration;
pub use namespaced_access_configuration::NamespacedAccessConfiguration;
pub use namespaced_access_rule_kind::NamespacedAccessRuleKind;
pub use per_user_group_rate_limiting_configuration::PerUserGroupRateLimitingConfiguration;
pub use port_range::{PortPolicy, PortSpec};
pub use rate_limiting_configuration::RateLimitingConfiguration;
pub use request_info::{RequestInfo, request_info};
pub use verb::Verb;

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

    /// Whether the verbs of `username` are restricted: some rule the caller is
    /// subject to lists `verbs`. Same definition as [`Self::restricts_ports`].
    #[must_use]
    pub fn restricts_verbs(&self, username: &str, groups: &[String]) -> bool {
        self.enabled
            && self
                .all_allowed_resources()
                .any(|rule| rule.restricts_verbs(username, groups))
    }

    /// The rules that accept the request: its path and, under a verb
    /// restriction, its verb. `info` is `None` when the verb could not be
    /// determined, which no rule accepts under a restriction.
    fn accepting_rules<'a>(
        &'a self,
        path: &'a str,
        info: Option<&'a RequestInfo>,
        username: &'a str,
        groups: &'a [String],
    ) -> impl Iterator<Item = &'a AllowedPathConfigurationEnum> + 'a {
        let restricted = self.restricts_verbs(username, groups);
        self.all_allowed_resources().filter(move |rule| {
            rule.matches(path, username, groups)
                && match info {
                    Some(info) => rule.allows_verb(info, restricted),
                    None => !restricted,
                }
        })
    }

    /// Whether some rule matches `path` whatever the verb: tells a refused
    /// verb from a refused path.
    #[must_use]
    pub fn matches_path(&self, path: &str, username: &str, groups: &[String]) -> bool {
        self.all_allowed_resources()
            .any(|rule| rule.matches(path, username, groups))
    }

    /// Check whether an upstream request may be forwarded.
    ///
    /// `path` is the path sent to the Kubernetes API (the
    /// `/clusters/{ns}/{cluster}` prefix already stripped) without its query
    /// string, `info` its [`request_info`] (`None` when it is an error). An
    /// empty allowed resource list, or a disabled security configuration,
    /// allows everything — that is the documented behaviour.
    #[must_use]
    pub fn is_request_allowed(
        &self,
        path: &str,
        info: Option<&RequestInfo>,
        username: &str,
        groups: &[String],
    ) -> bool {
        if !self.enabled || self.all_allowed_resources().next().is_none() {
            return true;
        }
        self.accepting_rules(path, info, username, groups)
            .next()
            .is_some()
    }

    /// Whether the ports of `username` are restricted: some rule the caller is
    /// subject to lists `allowed_ports`, whatever path it covers. A `Path` rule
    /// has no subject, so one `Path` rule with ports restricts every caller.
    #[must_use]
    pub fn restricts_ports(&self, username: &str, groups: &[String]) -> bool {
        self.enabled
            && self
                .all_allowed_resources()
                .any(|rule| rule.restricts_ports(username, groups))
    }

    /// Ports a request to `path` may open.
    ///
    /// `Any` when the caller is not restricted ([`Self::restricts_ports`]).
    /// Otherwise the union of the `allowed_ports` of the rules that accept the
    /// request and list ports: an accepting rule without `allowed_ports`
    /// grants nothing, so it can no longer lift the restriction.
    #[must_use]
    pub fn port_policy(
        &self,
        path: &str,
        info: Option<&RequestInfo>,
        username: &str,
        groups: &[String],
    ) -> PortPolicy {
        if !self.restricts_ports(username, groups) {
            return PortPolicy::Any;
        }
        let allowed = self
            .accepting_rules(path, info, username, groups)
            .filter_map(AllowedPathConfigurationEnum::port_policy)
            .flat_map(|policy| match policy {
                PortPolicy::Only(ranges) => ranges,
                PortPolicy::Any => Vec::new(),
            })
            .collect();
        PortPolicy::Only(allowed)
    }

    /// Whether `nodes/{node}/proxy` must be refused: the kubelet serves
    /// `/portForward`, `/exec` and `/run` there as a `get`, so any port or verb
    /// restriction closes it.
    #[must_use]
    pub fn restricts_node_proxy(&self, username: &str, groups: &[String]) -> bool {
        self.restricts_ports(username, groups) || self.restricts_verbs(username, groups)
    }

    /// Whether a `Path` rule that accepts the request names `nodes/{node}/proxy`
    /// explicitly: `nodes` and `proxy` as literal segments at their position,
    /// as in `/api/v1/nodes/*/proxy/**`. `nodes/**` or a `Crd` `Node` rule do
    /// not count, so a restriction keeps kubelet access closed unless asked for.
    #[must_use]
    pub fn node_proxy_explicitly_allowed(
        &self,
        path: &str,
        info: Option<&RequestInfo>,
        username: &str,
        groups: &[String],
    ) -> bool {
        self.accepting_rules(path, info, username, groups)
            .any(|rule| match rule {
                AllowedPathConfigurationEnum::Path(config) => names_node_proxy(&config.path),
                AllowedPathConfigurationEnum::Crd(_) => false,
            })
    }

    /// Reconcile-time warnings about rules a port or verb restriction changes
    /// the meaning of. Empty when no rule lists `allowed_ports` or `verbs`.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let ports_restricted = self
            .all_allowed_resources()
            .any(|rule| rule.port_policy().is_some());
        let verbs_restricted = self
            .all_allowed_resources()
            .any(|rule| rule.verbs().is_some());
        if !self.enabled || !(ports_restricted || verbs_restricted) {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        for (index, rule) in self.all_allowed_resources().enumerate() {
            let has_ports = rule.port_policy().is_some();
            let (covers_ports, covers_pod_subresources, covers_nodes) = match rule {
                AllowedPathConfigurationEnum::Path(config) => {
                    let segments: Vec<&str> =
                        config.path.split('/').filter(|s| !s.is_empty()).collect();
                    // `/api/**` or `/api/v1/**`: every core resource, pods included.
                    let core_wide = segments.first() == Some(&"api")
                        && segments.iter().take(3).any(|s| *s == "**");
                    // `pods/{name}/{subresource}` is reachable after `pods`.
                    let pod_subresources =
                        segments
                            .iter()
                            .position(|s| *s == "pods")
                            .is_some_and(|at| {
                                let rest = &segments[at + 1..];
                                rest.contains(&"**") || rest.len() >= 2
                            });
                    (
                        covers_port_paths(config),
                        core_wide || pod_subresources,
                        (core_wide || segments.contains(&"nodes"))
                            && !names_node_proxy(&config.path),
                    )
                }
                AllowedPathConfigurationEnum::Crd(config) => {
                    let core = config.group.is_empty();
                    (
                        core && (config.kind == "Pod" || config.kind == "Service"),
                        core && config.kind == "Pod",
                        core && config.kind == "Node",
                    )
                }
            };
            if ports_restricted && covers_ports && !has_ports {
                warnings.push(format!(
                    "rule {index} covers port-bearing paths without allowed_ports: it grants no port while ports are restricted"
                ));
            }
            if covers_nodes {
                warnings.push(format!(
                    "rule {index} covers nodes/ but not nodes/*/proxy explicitly: nodes/proxy stays denied"
                ));
            }
            if let Some(verbs) = rule.verbs()
                && covers_pod_subresources
            {
                let grants = |verb| verbs.iter().any(|v| v.grants(verb));
                if !grants(Verb::Create) && !has_ports {
                    warnings.push(format!(
                        "rule {index} never allows exec/attach/portforward, which are create"
                    ));
                }
                if grants(Verb::Patch) || grants(Verb::Update) {
                    warnings.push(format!(
                        "rule {index} also allows ephemeralcontainers (kubectl debug), which bypasses exec restrictions"
                    ));
                }
            }
        }
        warnings
    }
}

/// Whether a `Path` rule can match a port-bearing subresource
/// (`pods/{name}/portforward`, `pods|services/{name}:{port}/proxy`).
///
/// A parametised rule is tried, with the real matcher, against probe paths
/// in the namespace it names (or any): `*` and `**` count wherever they sit.
/// A literal rule matches one path, so that path itself is classified.
fn covers_port_paths(config: &AllowedPathConfiguration) -> bool {
    if !config.parametised {
        let segments: Vec<&str> = config.path.split('/').filter(|s| !s.is_empty()).collect();
        return matches!(segments.as_slice(), [.., "pods", _, "portforward"])
            || segments
                .windows(3)
                .any(|w| matches!(w[0], "pods" | "services") && w[2] == "proxy");
    }
    // Placeholders stand for one safe segment: any probe value will do.
    let pattern = config
        .path
        .replace("{{username}}", "probe")
        .replace("{{group}}", "probe");
    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    // The namespace and object name the pattern spells out, if any.
    let literal = |at: usize| segments.get(at).filter(|s| !s.contains('*')).copied();
    let namespace = match segments.get(2) {
        Some(&"namespaces") => literal(3).unwrap_or("probe"),
        _ => "probe",
    };
    let name = literal(5);
    let (forward, proxied) = (name.unwrap_or("probe"), name.unwrap_or("probe:1"));
    [
        format!("/api/v1/namespaces/{namespace}/pods/{forward}/portforward"),
        format!("/api/v1/namespaces/{namespace}/pods/{proxied}/proxy"),
        format!("/api/v1/namespaces/{namespace}/services/{proxied}/proxy"),
    ]
    .iter()
    .any(|probe| path_matcher::path_matches_pattern(&pattern, probe))
}

/// Whether a `Path` pattern names `/api/{version}/nodes/{node}/proxy`.
fn names_node_proxy(pattern: &str) -> bool {
    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    matches!(segments.as_slice(), ["api", _, "nodes", _, "proxy", ..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checks for a `GET` (verb computed as the proxy does).
    trait GetRequest {
        fn get_allowed(&self, path: &str, username: &str, groups: &[String]) -> bool;
        fn get_port_policy(&self, path: &str, username: &str, groups: &[String]) -> PortPolicy;
        fn get_node_proxy(&self, path: &str, username: &str, groups: &[String]) -> bool;
    }

    impl GetRequest for SecurityConfiguration {
        fn get_allowed(&self, path: &str, username: &str, groups: &[String]) -> bool {
            let info = request_info("GET", path, "").ok();
            self.is_request_allowed(path, info.as_ref(), username, groups)
        }
        fn get_port_policy(&self, path: &str, username: &str, groups: &[String]) -> PortPolicy {
            let info = request_info("GET", path, "").ok();
            self.port_policy(path, info.as_ref(), username, groups)
        }
        fn get_node_proxy(&self, path: &str, username: &str, groups: &[String]) -> bool {
            let info = request_info("GET", path, "").ok();
            self.node_proxy_explicitly_allowed(path, info.as_ref(), username, groups)
        }
    }

    fn path_rule(path: &str, parametised: bool) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Path(AllowedPathConfiguration {
            path: path.to_string(),
            parametised,
            allowed_ports: None,
            verbs: None,
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
            verbs: None,
        })
    }

    const PORT_FORWARD: &str = "/api/v1/namespaces/dev/pods/web/portforward";

    fn verb_rule(path: &str, verbs: &[Verb]) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Path(AllowedPathConfiguration {
            path: path.to_string(),
            parametised: true,
            allowed_ports: None,
            verbs: Some(verbs.to_vec()),
        })
    }

    /// Whether `method` on `path?query` passes the allow-list.
    fn allowed(config: &SecurityConfiguration, method: &str, path: &str, query: &str) -> bool {
        let info = request_info(method, path, query).ok();
        config.is_request_allowed(path, info.as_ref(), "alice", &[])
    }

    /// RFC 0006 §1: pods read-only, a ConfigMap rule without verbs.
    fn verb_restricted() -> SecurityConfiguration {
        let mut configmaps = crd_pod(None);
        if let AllowedPathConfigurationEnum::Crd(config) = &mut configmaps {
            config.kind = "ConfigMap".to_string();
            config.plural = Some("configmaps".to_string());
        }
        SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                verb_rule("/api/v1/namespaces/dev/pods/**", &Verb::READ),
                configmaps,
            ],
            ..SecurityConfiguration::default()
        }
    }

    #[test]
    fn a_rule_with_verbs_limits_what_it_allows() {
        let config = verb_restricted();
        const PODS: &str = "/api/v1/namespaces/dev/pods";
        assert!(allowed(&config, "GET", PODS, ""));
        assert!(allowed(&config, "GET", PODS, "watch=true"));
        assert!(allowed(&config, "GET", &format!("{PODS}/web"), ""));
        assert!(!allowed(&config, "DELETE", &format!("{PODS}/web"), ""));
        assert!(!allowed(&config, "POST", PODS, ""));
        assert!(!allowed(
            &config,
            "PATCH",
            &format!("{PODS}/web/ephemeralcontainers"),
            ""
        ));
        // T2: a websocket exec is a GET, but create.
        assert!(!allowed(
            &config,
            "GET",
            &format!("{PODS}/web/exec"),
            "command=sh"
        ));
        // Undeterminable verbs are refused under the restriction.
        assert!(!allowed(&config, "OPTIONS", PODS, ""));
    }

    /// T9: under a verb restriction, a rule without verbs only grants reads.
    #[test]
    fn a_rule_without_verbs_is_read_only_under_a_restriction() {
        let config = verb_restricted();
        const CM: &str = "/api/v1/namespaces/dev/configmaps/app";
        assert!(allowed(&config, "GET", CM, ""));
        assert!(!allowed(&config, "DELETE", CM, ""));
        assert!(!allowed(&config, "PUT", CM, ""));
    }

    #[test]
    fn without_any_verbs_every_verb_passes_as_before() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/namespaces/dev/**", true)],
            ..SecurityConfiguration::default()
        };
        assert!(!config.restricts_verbs("alice", &[]));
        for method in ["GET", "DELETE", "POST", "OPTIONS"] {
            assert!(
                allowed(&config, method, "/api/v1/namespaces/dev/pods", ""),
                "{method}"
            );
        }
    }

    #[test]
    fn star_allows_every_verb() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![verb_rule("/api/v1/namespaces/dev/**", &[Verb::All])],
            ..SecurityConfiguration::default()
        };
        assert!(allowed(
            &config,
            "DELETE",
            "/api/v1/namespaces/dev/pods",
            ""
        ));
        assert!(!allowed(
            &config,
            "OPTIONS",
            "/api/v1/namespaces/dev/pods",
            ""
        ));
    }

    /// `allowed_ports` implies `create` on `portforward`, not on `proxy`.
    #[test]
    fn allowed_ports_implies_create_on_portforward_only() {
        let mut config = verb_restricted();
        config.allowed_resources.push(port_forward_rule(
            "/api/v1/namespaces/dev/pods/*/**",
            Some(&["8080"]),
        ));
        assert!(allowed(&config, "GET", PORT_FORWARD, "ports=8080"));
        let info = request_info("GET", PORT_FORWARD, "").ok();
        assert!(
            config
                .port_policy(PORT_FORWARD, info.as_ref(), "alice", &[])
                .allows(8080)
        );
        assert!(!allowed(
            &config,
            "POST",
            "/api/v1/namespaces/dev/pods/web:8080/proxy",
            ""
        ));
        assert!(allowed(
            &config,
            "GET",
            "/api/v1/namespaces/dev/pods/web:8080/proxy",
            ""
        ));
    }

    /// The port policy only counts the rules that accepted path and verb.
    #[test]
    fn the_port_policy_ignores_rules_refusing_the_verb() {
        let mut ports =
            port_forward_rule("/api/v1/namespaces/dev/services/*/proxy/**", Some(&["80"]));
        if let AllowedPathConfigurationEnum::Path(config) = &mut ports {
            config.verbs = Some(vec![Verb::Get]);
        }
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![ports],
            ..SecurityConfiguration::default()
        };
        const PROXY: &str = "/api/v1/namespaces/dev/services/web:80/proxy/x";
        let get = request_info("GET", PROXY, "").ok();
        let post = request_info("POST", PROXY, "").ok();
        assert!(
            config
                .port_policy(PROXY, get.as_ref(), "alice", &[])
                .allows(80)
        );
        assert!(
            !config
                .port_policy(PROXY, post.as_ref(), "alice", &[])
                .allows(80)
        );
    }

    /// T8: a verb restriction alone closes nodes/proxy unless named.
    #[test]
    fn a_verb_restriction_closes_node_proxy() {
        let mut config = verb_restricted();
        config
            .allowed_resources
            .push(path_rule("/api/v1/nodes/**", true));
        assert!(config.restricts_node_proxy("alice", &[]));
        assert!(!config.restricts_ports("alice", &[]));
        const NODE: &str = "/api/v1/nodes/n1/proxy/exec/dev/web/app";
        assert!(allowed(&config, "GET", NODE, "command=sh"));
        assert!(!config.get_node_proxy(NODE, "alice", &[]));
        config
            .allowed_resources
            .push(path_rule("/api/v1/nodes/*/proxy/**", true));
        assert!(config.get_node_proxy(NODE, "alice", &[]));
    }

    #[test]
    fn verb_warnings_flag_exec_and_ephemeral_containers() {
        let mut pod_write = crd_pod(None);
        if let AllowedPathConfigurationEnum::Crd(config) = &mut pod_write {
            config.verbs = Some(vec![Verb::Get, Verb::Patch]);
        }
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                verb_rule("/api/v1/namespaces/dev/pods/**", &Verb::READ),
                pod_write,
                verb_rule("/api/v1/namespaces/dev/pods", &[Verb::List]),
                path_rule("/api/v1/nodes/**", true),
            ],
            ..SecurityConfiguration::default()
        };
        assert_eq!(
            config.warnings(),
            vec![
                "rule 0 never allows exec/attach/portforward, which are create",
                "rule 1 never allows exec/attach/portforward, which are create",
                "rule 1 also allows ephemeralcontainers (kubectl debug), which bypasses exec restrictions",
                "rule 3 covers nodes/ but not nodes/*/proxy explicitly: nodes/proxy stays denied",
            ]
        );
    }

    fn crd_pod(ports: Option<&[&str]>) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Crd(AllowedCrdConfiguration {
            group: String::new(),
            version: "v1".to_string(),
            kind: "Pod".to_string(),
            plural: Some("pods".to_string()),
            namespace: NamespacedAccessConfiguration {
                enabled: false,
                rule_kind: NamespacedAccessRuleKind::AllowedNamespaces(vec![]),
            },
            namespaced: true,
            allowed_ports: ports.map(|ports| {
                ports
                    .iter()
                    .map(|port| PortSpec((*port).to_string()))
                    .collect()
            }),
            verbs: None,
        })
    }

    #[test]
    fn port_forward_is_unrestricted_without_rules() {
        let config = SecurityConfiguration::default();
        assert_eq!(
            config.get_port_policy(PORT_FORWARD, "alice", &[]),
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

        let policy = config.get_port_policy(PORT_FORWARD, "alice", &[]);
        assert!(policy.allows(8080));
        assert!(policy.allows(9001));
        assert!(!policy.allows(22));
    }

    /// T1: under a restriction, a matching rule without `allowed_ports` grants
    /// nothing instead of lifting the restriction (R2).
    #[test]
    fn a_matching_rule_without_ports_grants_no_port() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/dev/pods/*/portforward", Some(&["8080"])),
                port_forward_rule("/api/v1/namespaces/dev/**", None),
                crd_pod(None),
            ],
            ..SecurityConfiguration::default()
        };
        let policy = config.get_port_policy(PORT_FORWARD, "alice", &[]);
        assert!(policy.allows(8080));
        assert!(!policy.allows(5432));
        // The rules still allow their other paths.
        assert!(config.get_allowed("/api/v1/namespaces/dev/pods/web/log", "alice", &[]));
    }

    /// Without any rule listing ports, nothing is restricted, as before.
    #[test]
    fn rules_without_ports_restrict_nothing() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/dev/**", None),
                crd_pod(None),
            ],
            ..SecurityConfiguration::default()
        };
        assert!(!config.restricts_ports("alice", &[]));
        assert_eq!(
            config.get_port_policy(PORT_FORWARD, "alice", &[]),
            PortPolicy::Any
        );
    }

    /// A `Path` rule with ports restricts every caller, whatever it covers.
    #[test]
    fn a_path_rule_with_ports_restricts_everyone() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/other/pods/*/portforward", Some(&["80"])),
                crd_pod(None),
            ],
            ..SecurityConfiguration::default()
        };
        assert!(config.restricts_ports("bob", &[]));
        assert_eq!(
            config.get_port_policy(PORT_FORWARD, "bob", &[]),
            PortPolicy::Only(Vec::new())
        );
    }

    /// A `Crd` rule grants its ports, and only on the paths it matches.
    #[test]
    fn a_crd_rule_grants_its_ports() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![crd_pod(Some(&["8080"]))],
            ..SecurityConfiguration::default()
        };
        assert!(
            config
                .get_port_policy(PORT_FORWARD, "alice", &[])
                .allows(8080)
        );
        assert!(
            !config
                .get_port_policy(
                    "/api/v1/namespaces/dev/services/web:8080/proxy",
                    "alice",
                    &[]
                )
                .allows(8080),
            "a pods rule never grants a Service port"
        );
    }

    /// A `Crd` rule whose parametised namespace expands to nothing for the
    /// caller does not restrict them.
    #[test]
    fn a_crd_rule_restricts_only_the_callers_it_applies_to() {
        let mut rule = crd_pod(Some(&["8080"]));
        if let AllowedPathConfigurationEnum::Crd(config) = &mut rule {
            config.namespace = NamespacedAccessConfiguration {
                enabled: true,
                rule_kind: NamespacedAccessRuleKind::ParametisedRule("team-{{group}}".to_string()),
            };
        }
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![rule],
            ..SecurityConfiguration::default()
        };
        assert!(config.restricts_ports("alice", &["a".to_string()]));
        assert!(!config.restricts_ports("bob", &[]));
    }

    #[test]
    fn node_proxy_needs_an_explicit_path_rule() {
        const NODE: &str = "/api/v1/nodes/n1/proxy/portForward/dev/web";
        let explicit = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/nodes/*/proxy/**", true)],
            ..SecurityConfiguration::default()
        };
        assert!(explicit.get_node_proxy(NODE, "alice", &[]));
        for broad in ["/api/v1/nodes/**", "/api/**"] {
            let config = SecurityConfiguration {
                enabled: true,
                allowed_resources: vec![path_rule(broad, true)],
                ..SecurityConfiguration::default()
            };
            assert!(config.get_allowed(NODE, "alice", &[]), "{broad}");
            assert!(!config.get_node_proxy(NODE, "alice", &[]), "{broad}");
        }
    }

    #[test]
    fn only_port_bearing_paths_are_flagged() {
        let rule = |path: &str, parametised: bool| AllowedPathConfiguration {
            path: path.to_string(),
            parametised,
            allowed_ports: None,
            verbs: None,
        };
        for (pattern, parametised, covers) in [
            ("/api/v1/namespaces/dev/pods/**", true, true),
            ("/api/v1/namespaces/dev/pods/*/portforward", true, true),
            ("/api/v1/namespaces/dev/services/*/proxy/**", true, true),
            ("/api/v1/namespaces/dev/pods/web/*", true, true),
            // A wildcard before the resource reaches pods too.
            ("/api/v1/namespaces/dev/**", true, true),
            ("/api/v1/namespaces/*/*/*/portforward", true, true),
            ("/api/**", true, true),
            ("/api/v1/namespaces/{{username}}/pods/**", true, true),
            ("/api/v1/namespaces/dev/pods/web/portforward", false, true),
            ("/api/v1/namespaces/dev/pods/*/exec", true, false),
            ("/api/v1/namespaces/dev/pods/*/log", true, false),
            ("/api/v1/namespaces/dev/pods", true, false),
            ("/api/v1/namespaces/dev/configmaps/**", true, false),
            ("/apis/apps/v1/**", true, false),
            ("/api/v1/namespaces/dev/pods/web/exec", false, false),
        ] {
            assert_eq!(
                covers_port_paths(&rule(pattern, parametised)),
                covers,
                "{pattern} (parametised: {parametised})"
            );
        }
    }

    #[test]
    fn warnings_flag_rules_a_restriction_changes() {
        let mut node = crd_pod(None);
        if let AllowedPathConfigurationEnum::Crd(config) = &mut node {
            config.kind = "Node".to_string();
            config.plural = Some("nodes".to_string());
            config.namespaced = false;
        }
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![
                port_forward_rule("/api/v1/namespaces/dev/pods/*/portforward", Some(&["8080"])),
                crd_pod(None),
                path_rule("/api/v1/nodes/**", true),
                node,
                path_rule("/api/v1/nodes/*/proxy/**", true),
                path_rule("/apis/apps/v1/**", true),
            ],
            ..SecurityConfiguration::default()
        };
        assert_eq!(
            config.warnings(),
            vec![
                "rule 1 covers port-bearing paths without allowed_ports: it grants no port while ports are restricted",
                "rule 2 covers nodes/ but not nodes/*/proxy explicitly: nodes/proxy stays denied",
                "rule 3 covers nodes/ but not nodes/*/proxy explicitly: nodes/proxy stays denied",
            ]
        );
        let unrestricted = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![crd_pod(None), path_rule("/api/v1/nodes/**", true)],
            ..SecurityConfiguration::default()
        };
        assert!(unrestricted.warnings().is_empty());
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
            config.get_port_policy(PORT_FORWARD, "alice", &[]),
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
            assert!(!config.get_allowed(path, "alice", &[]), "{path}");
        }
    }

    #[test]
    fn empty_allow_list_allows_everything() {
        let config = SecurityConfiguration::default();
        assert!(config.get_allowed("/api/v1/namespaces/kube-system/secrets", "alice", &[]));
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
        assert!(config.get_allowed("/api/v1/pods", "alice", &[]));
        assert!(!config.get_allowed("/api/v1/secrets", "alice", &[]));
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
        assert!(config.get_allowed("/api/v1/pods", "alice", &[]));
        assert!(config.get_allowed("/api/v1/configmaps", "alice", &[]));
        assert!(!config.get_allowed("/api/v1/secrets", "alice", &[]));
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
        assert!(config.get_allowed("/api/v1/namespaces/kube-system/secrets", "alice", &[]));
    }

    #[test]
    fn non_matching_path_is_rejected() {
        let config = SecurityConfiguration {
            enabled: true,
            allowed_resources: vec![path_rule("/api/v1/namespaces/{{group}}/pods/**", true)],
            ..SecurityConfiguration::default()
        };
        let groups = vec!["dev-alice".to_string()];

        assert!(config.get_allowed("/api/v1/namespaces/dev-alice/pods", "alice", &groups));
        assert!(config.get_allowed(
            "/api/v1/namespaces/dev-alice/pods/mypod/log",
            "alice",
            &groups
        ));
        assert!(!config.get_allowed("/api/v1/namespaces/kube-system/secrets", "alice", &groups));
    }

    fn crd_rule(namespace: NamespacedAccessConfiguration) -> AllowedPathConfigurationEnum {
        AllowedPathConfigurationEnum::Crd(AllowedCrdConfiguration {
            group: "example.com".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: Some("widgets".to_string()),
            namespace,
            namespaced: true,
            allowed_ports: None,
            verbs: None,
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

        assert!(config.get_allowed("/apis/example.com/v1/namespaces/dev/widgets", "alice", &[]));
        // Right resource, wrong (denied) namespace.
        assert!(!config.get_allowed(
            "/apis/example.com/v1/namespaces/kube-system/widgets",
            "alice",
            &[]
        ));
        // Different resource entirely.
        assert!(!config.get_allowed("/api/v1/namespaces/dev/secrets", "alice", &[]));
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
        assert!(config.get_allowed("/api/v1/namespaces/dev/pods", "alice", &[]));
        assert!(config.get_allowed(
            "/apis/example.com/v1/namespaces/anything/widgets",
            "alice",
            &[]
        ));
        assert!(!config.get_allowed("/api/v1/namespaces/dev/secrets", "alice", &[]));
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
        assert!(config.get_allowed("/api/v1/namespaces/dev/pods", "alice", &[]));
        assert!(config.get_allowed("/apis/apps/v1/namespaces/staging/deployments", "alice", &[]));
        assert!(!config.get_allowed("/api/v1/namespaces/dev/secrets", "alice", &[]));
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
