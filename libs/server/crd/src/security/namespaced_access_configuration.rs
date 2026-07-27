use crate::default::default_disabled;
use crate::security::path_matcher::{expand_parametised_patterns, path_matches_pattern};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::namespaced_access_rule_kind::NamespacedAccessRuleKind;

/// Whether or not the rules restrict access to certains namespaces, if true, the allowed paths will be restricted to the namespaces specified in the allowed paths configuration
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NamespacedAccessConfiguration {
    /// If the feature is enabled
    /// default: false
    #[serde(default = "default_disabled")]
    pub enabled: bool,

    /// The kind of the namespace access rule
    pub rule_kind: NamespacedAccessRuleKind,
}

impl NamespacedAccessConfiguration {
    /// Whether the caller may reach `namespace`.
    ///
    /// A disabled rule imposes no restriction (allow). Otherwise:
    /// - `AllowedNamespaces`: allow iff the namespace is listed (default-deny);
    /// - `DeniedNamespaces`: allow unless the namespace is listed;
    /// - `ParametisedRule`: expand `{{username}}`/`{{group}}` with literal-safe
    ///   substitution, then match the single namespace segment against each
    ///   candidate (so `dev-{{username}}` or `dev-*` work as expected).
    #[must_use]
    pub fn is_namespace_allowed(&self, namespace: &str, username: &str, groups: &[String]) -> bool {
        if !self.enabled {
            return true;
        }
        match &self.rule_kind {
            NamespacedAccessRuleKind::AllowedNamespaces(list) => {
                list.iter().any(|allowed| allowed == namespace)
            }
            NamespacedAccessRuleKind::DeniedNamespaces(list) => {
                !list.iter().any(|denied| denied == namespace)
            }
            NamespacedAccessRuleKind::ParametisedRule(pattern) => {
                expand_parametised_patterns(pattern, username, groups)
                    .iter()
                    .any(|candidate| path_matches_pattern(candidate, namespace))
            }
        }
    }

    /// Reject a parametised rule that uses an unknown placeholder.
    pub fn validate(&self) -> Result<(), String> {
        if let NamespacedAccessRuleKind::ParametisedRule(pattern) = &self.rule_kind {
            for capture in super::allowed_path_configuration::mustache_captures(pattern) {
                if capture != "username" && capture != "group" {
                    return Err(format!(
                        "Invalid parameter in namespace rule: {capture}, allowed parameters are {{username}} and {{group}}"
                    ));
                }
            }
        }
        Ok(())
    }
}
