use crate::default::default_disabled;
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
