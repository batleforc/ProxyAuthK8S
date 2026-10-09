use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::allowed_crd_configuration::AllowedCrdConfiguration;
use super::allowed_path_configuration::AllowedPathConfiguration;
use super::port_range::PortPolicy;

/// How an allowed resource is described.
///
/// - `Path` matches on the raw upstream path (with `*`/`**`/`{{...}}` semantics);
/// - `Crd` matches on group/version/kind and applies the per-namespace access
///   rules, translating to a path via [`super::AllowedCrdConfiguration`].
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum AllowedPathConfigurationEnum {
    Path(AllowedPathConfiguration),
    Crd(AllowedCrdConfiguration),
}

impl AllowedPathConfigurationEnum {
    /// Check whether an upstream request path is allowed by this rule.
    #[must_use]
    pub fn matches(&self, path: &str, username: &str, groups: &[String]) -> bool {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.matches(path, username, groups),
            AllowedPathConfigurationEnum::Crd(config) => config.matches(path, username, groups),
        }
    }

    /// Ports a port-forward matched by this rule may open. `Crd` rules do not
    /// restrict ports.
    #[must_use]
    pub fn port_policy(&self) -> PortPolicy {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.port_policy(),
            AllowedPathConfigurationEnum::Crd(_) => PortPolicy::Any,
        }
    }

    /// Validate the rule at reconcile time (mirrors the CEL admission rules).
    pub fn validate(&self) -> Result<(), String> {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.validate(),
            AllowedPathConfigurationEnum::Crd(config) => config.validate(),
        }
    }
}
