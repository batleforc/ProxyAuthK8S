use crate::default::default_enabled;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::namespaced_access_configuration::NamespacedAccessConfiguration;
use super::port_range::{PortPolicy, PortSpec};
use super::verb::{VERBS_RULE, Verb, validate_verbs};

/// Kinds whose port-bearing subresources (`portforward`, `{name}:{port}/proxy`)
/// an `allowed_ports` list can apply to. The CEL rule below mirrors it.
const PORT_KINDS: [&str; 2] = ["Pod", "Service"];

/// Allowed crd configuration, used in conjunction with the `allowed_paths` configuration
/// /apis/{group}/{version}/namespaces/{namespace}/{kind}/
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": "!has(self.allowed_ports) || self.kind in ['Pod', 'Service']",
        "message": "allowed_ports only applies to the Pod and Service kinds",
    }),
]))]
pub struct AllowedCrdConfiguration {
    /// The group of the crd
    pub group: String,
    /// The version of the crd
    pub version: String,
    /// The kind of the crd
    pub kind: String,
    /// Wether or not the kind has a specific plural form, if true, the plural form will be used in the path instead of the kind
    /// for example, if the kind is "`MyResource`" and the plural form is "`MyResources`"
    /// the path will be /apis/{group}/{version}/namespaces/{namespace}/myresources/
    /// instead of       /apis/{group}/{version}/namespaces/{namespace}/myresource/
    /// In case of cluster-scoped crd
    /// the path will be /apis/{group}/{version}/myresources/
    /// instead of       /apis/{group}/{version}/myresource/
    pub plural: Option<String>,

    /// Wether or not the ressource is namespaced, if true, the namespace access rules will be applied to this resource
    pub namespace: NamespacedAccessConfiguration,
    /// Namespaced or not
    /// default: true
    #[serde(default = "default_enabled")]
    pub namespaced: bool,

    /// Ports this rule grants on the port-bearing subresources of a `Pod`
    /// (`portforward`, `{name}:{port}/proxy`) or a `Service`
    /// (`{name}:{port}/proxy`, a Service port, not its `targetPort`), as
    /// single ports (`"8080"`) or inclusive ranges (`"9000-9100"`)
    ///
    /// Same meaning as on a `Path` rule: once any rule a caller is subject to
    /// lists ports, a rule without `allowed_ports` grants no port at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub allowed_ports: Option<Vec<PortSpec>>,

    /// Kubernetes verbs this rule allows on the resource and all its
    /// subresources (same values and meaning as on a `Path` rule)
    ///
    /// A `Pod` rule with `patch` or `update` also allows
    /// `ephemeralcontainers`, that is `kubectl debug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 9), extend("x-kubernetes-validations" = [
        serde_json::json!({
            "rule": VERBS_RULE,
            "message": "\"*\" already allows every verb and must stand alone",
        }),
    ]))]
    pub verbs: Option<Vec<Verb>>,
}

impl AllowedCrdConfiguration {
    /// The resource segment used in the request path: the explicit plural if set,
    /// otherwise the lower-cased kind (a best-effort default).
    fn resource_segment(&self) -> String {
        self.plural
            .clone()
            .unwrap_or_else(|| self.kind.to_lowercase())
    }

    /// Whether an upstream request `path` addresses this CRD, honouring the
    /// namespace access rules when the resource is namespaced.
    ///
    /// Recognised shapes (core group `""` uses `/api/{version}`, others use
    /// `/apis/{group}/{version}`):
    /// - namespaced: `.../{version}/namespaces/{ns}/{plural}[/...]`
    /// - namespaced, cluster-wide list: `.../{version}/{plural}[/...]`
    /// - cluster-scoped: `.../{version}/{plural}[/...]`
    ///
    /// A namespaced resource reached cluster-wide (no `/namespaces/{ns}/`) is
    /// only allowed when no namespace restriction is in force, since a
    /// cluster-wide list would otherwise return namespaces the rule denies.
    #[must_use]
    pub fn matches(&self, path: &str, username: &str, groups: &[String]) -> bool {
        // This matcher reads the namespace at a fixed position and ignores every
        // segment past the resource. A `..` (or encoded-separator) segment would
        // therefore let it authorize one namespace while the url-normalized
        // upstream request resolves to another — e.g.
        // `.../namespaces/dev/widgets/../../prod/widgets` is judged as `dev` but
        // reaches `prod`. Apply the same traversal guard the `Path` matcher uses.
        if !crate::security::path_matcher::path_has_no_traversal(path) {
            return false;
        }
        // Compare the segments the way the upstream apiserver will see them:
        // it percent-decodes the path once, so `namespaces/kube%2Dsystem` reaches
        // `kube-system`. Matching the raw segment would let an encoded spelling
        // slip past a `DeniedNamespaces` rule.
        let decoded: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(crate::security::path_matcher::percent_decode_once)
            .collect();
        let segments: Vec<&str> = decoded.iter().map(String::as_str).collect();
        let mut idx = 0;

        // API group prefix.
        match segments.first().copied() {
            Some("apis") if !self.group.is_empty() => {
                idx += 1;
                if segments.get(idx).copied() != Some(self.group.as_str()) {
                    return false;
                }
                idx += 1;
            }
            Some("api") if self.group.is_empty() => {
                idx += 1;
            }
            _ => return false,
        }

        // Version.
        if segments.get(idx).copied() != Some(self.version.as_str()) {
            return false;
        }
        idx += 1;

        let resource = self.resource_segment();

        if self.namespaced {
            let mut namespace: Option<&str> = None;
            if segments.get(idx).copied() == Some("namespaces") {
                // `.../namespaces/{ns}/{plural}`; the ns must be followed by the
                // resource, otherwise this is a different path (e.g. GET on the
                // namespace object itself).
                namespace = match segments.get(idx + 1).copied() {
                    Some(ns) => Some(ns),
                    None => return false,
                };
                idx += 2;
            }
            if segments.get(idx).copied() != Some(resource.as_str()) {
                return false;
            }
            match namespace {
                Some(ns) => self.namespace.is_namespace_allowed(ns, username, groups),
                // Cluster-wide access to a namespaced resource: allow only when
                // no namespace restriction applies.
                None => !self.namespace.enabled,
            }
        } else {
            // Cluster-scoped: the resource segment follows the version directly.
            segments.get(idx).copied() == Some(resource.as_str())
        }
    }

    /// The ports this rule grants, `None` when it lists none.
    ///
    /// Invalid entries (refused by [`Self::validate`]) grant nothing.
    #[must_use]
    pub fn port_policy(&self) -> Option<PortPolicy> {
        self.allowed_ports.as_ref().map(|ports| {
            PortPolicy::Only(ports.iter().filter_map(|port| port.range().ok()).collect())
        })
    }

    /// Whether this rule can apply to the caller at all: a parametised
    /// namespace rule that expands to no candidate (a `{{group}}` rule for a
    /// caller without groups) reaches nothing.
    #[must_use]
    pub fn applies_to(&self, username: &str, groups: &[String]) -> bool {
        self.namespace.applies_to(username, groups)
    }

    /// Reject a configuration whose parametised namespace rule uses an unknown
    /// placeholder, or whose ports are invalid or set on a kind without ports.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(verbs) = &self.verbs {
            validate_verbs(verbs)?;
        }
        if let Some(ports) = &self.allowed_ports {
            if !PORT_KINDS.contains(&self.kind.as_str()) {
                return Err(format!(
                    "allowed_ports only applies to the Pod and Service kinds, not {}",
                    self.kind
                ));
            }
            for port in ports {
                port.range()?;
            }
        }
        self.namespace.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::NamespacedAccessRuleKind;

    fn ns_rule(
        enabled: bool,
        rule_kind: NamespacedAccessRuleKind,
    ) -> NamespacedAccessConfiguration {
        NamespacedAccessConfiguration { enabled, rule_kind }
    }

    fn open_ns() -> NamespacedAccessConfiguration {
        // `enabled = false` → no namespace restriction.
        ns_rule(false, NamespacedAccessRuleKind::AllowedNamespaces(vec![]))
    }

    fn widget(
        namespace: NamespacedAccessConfiguration,
        namespaced: bool,
    ) -> AllowedCrdConfiguration {
        AllowedCrdConfiguration {
            group: "example.com".to_string(),
            version: "v1".to_string(),
            kind: "Widget".to_string(),
            plural: Some("widgets".to_string()),
            namespace,
            namespaced,
            allowed_ports: None,
            verbs: None,
        }
    }

    fn groups() -> Vec<String> {
        vec!["dev".to_string()]
    }

    #[test]
    fn matches_a_namespaced_custom_resource_and_its_subresources() {
        let rule = widget(open_ns(), true);
        assert!(rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets",
            "alice",
            &groups()
        ));
        assert!(rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets/foo/status",
            "alice",
            &groups()
        ));
        // Cluster-wide list is allowed when no namespace restriction is set.
        assert!(rule.matches("/apis/example.com/v1/widgets", "alice", &groups()));
    }

    #[test]
    fn rejects_wrong_group_version_or_resource() {
        let rule = widget(open_ns(), true);
        assert!(!rule.matches(
            "/apis/other.com/v1/namespaces/dev/widgets",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/apis/example.com/v2/namespaces/dev/widgets",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/dev/gadgets",
            "alice",
            &groups()
        ));
        // Core-group path must not match a non-core rule.
        assert!(!rule.matches("/api/v1/namespaces/dev/widgets", "alice", &groups()));
    }

    #[test]
    fn allowed_namespaces_are_default_deny() {
        let rule = widget(
            ns_rule(
                true,
                NamespacedAccessRuleKind::AllowedNamespaces(vec!["dev".to_string()]),
            ),
            true,
        );
        assert!(rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/prod/widgets",
            "alice",
            &groups()
        ));
        // Cluster-wide access is refused once a restriction is in force.
        assert!(!rule.matches("/apis/example.com/v1/widgets", "alice", &groups()));
    }

    #[test]
    fn denied_namespaces_block_only_the_listed_ones() {
        let rule = widget(
            ns_rule(
                true,
                NamespacedAccessRuleKind::DeniedNamespaces(vec!["kube-system".to_string()]),
            ),
            true,
        );
        assert!(rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/kube-system/widgets",
            "alice",
            &groups()
        ));
        // The apiserver decodes the path, so an encoded spelling of a denied
        // namespace must be denied too.
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/kube%2Dsystem/widgets",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/%6bube-system/widgets",
            "alice",
            &groups()
        ));
    }

    #[test]
    fn parametised_namespace_rule_binds_to_the_caller() {
        let rule = widget(
            ns_rule(
                true,
                NamespacedAccessRuleKind::ParametisedRule("dev-{{username}}".to_string()),
            ),
            true,
        );
        assert!(rule.matches(
            "/apis/example.com/v1/namespaces/dev-alice/widgets",
            "alice",
            &groups()
        ));
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/dev-bob/widgets",
            "alice",
            &groups()
        ));
        // A `*` username cannot self-escalate through the parametised rule.
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/dev-x/widgets",
            "*",
            &groups()
        ));
    }

    #[test]
    fn core_group_and_plural_default_are_handled() {
        // Core group: /api/{version}/...
        let pod = AllowedCrdConfiguration {
            group: String::new(),
            version: "v1".to_string(),
            kind: "Pod".to_string(),
            plural: Some("pods".to_string()),
            namespace: open_ns(),
            namespaced: true,
            allowed_ports: None,
            verbs: None,
        };
        assert!(pod.matches("/api/v1/namespaces/dev/pods", "alice", &groups()));
        assert!(!pod.matches(
            "/apis/example.com/v1/namespaces/dev/pods",
            "alice",
            &groups()
        ));

        // Plural defaults to the lower-cased kind when not provided.
        let no_plural = AllowedCrdConfiguration {
            plural: None,
            ..widget(open_ns(), true)
        };
        assert!(no_plural.matches(
            "/apis/example.com/v1/namespaces/dev/widget",
            "alice",
            &groups()
        ));
    }

    #[test]
    fn cluster_scoped_resource_rejects_a_namespaced_path() {
        let rule = widget(open_ns(), false);
        assert!(rule.matches("/apis/example.com/v1/widgets", "alice", &groups()));
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets",
            "alice",
            &groups()
        ));
    }

    #[test]
    fn traversal_segments_cannot_escape_the_namespace_confinement() {
        // Confined to `dev`; a `..` climb that the url layer normalizes to `prod`
        // must be denied, not silently ignored by the positional parser.
        let rule = widget(
            ns_rule(
                true,
                NamespacedAccessRuleKind::AllowedNamespaces(vec!["dev".to_string()]),
            ),
            true,
        );
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets/../../prod/widgets",
            "alice",
            &groups()
        ));
        // Encoded separators are refused too.
        assert!(!rule.matches(
            "/apis/example.com/v1/namespaces/dev/widgets/..%2f..%2fprod%2fwidgets",
            "alice",
            &groups()
        ));
    }

    /// The CEL kind rule and `validate()` must agree on which kinds take ports.
    #[test]
    fn allowed_ports_is_limited_to_pods_and_services() {
        let schema =
            serde_json::to_string(&schemars::schema_for!(AllowedCrdConfiguration)).unwrap();
        for kind in PORT_KINDS {
            assert!(
                schema.contains(&format!("'{kind}'")),
                "CEL rule misses {kind}"
            );
        }
        for (kind, ok) in [
            ("Pod", true),
            ("Service", true),
            ("Node", false),
            ("Widget", false),
        ] {
            let rule = AllowedCrdConfiguration {
                kind: kind.to_string(),
                allowed_ports: Some(vec![PortSpec("8080".to_string())]),
                ..widget(open_ns(), true)
            };
            assert_eq!(rule.validate().is_ok(), ok, "{kind}");
        }
        let invalid = AllowedCrdConfiguration {
            kind: "Pod".to_string(),
            allowed_ports: Some(vec![PortSpec("0".to_string())]),
            ..widget(open_ns(), true)
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn validate_rejects_unknown_placeholder_in_namespace_rule() {
        let rule = widget(
            ns_rule(
                true,
                NamespacedAccessRuleKind::ParametisedRule("{{tenant}}".to_string()),
            ),
            true,
        );
        assert!(rule.validate().is_err());
    }
}
