use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

//use super::allowed_crd_configuration::AllowedCrdConfiguration;
use super::allowed_path_configuration::AllowedPathConfiguration;

/// How an allowed resource is described.
///
/// Only path rules are implemented. A `Crd` variant matching on
/// group/version/kind is described by [`super::AllowedCrdConfiguration`] and
/// tracked on the roadmap; adding a variant here is the only change the proxy
/// needs, since matching goes through [`AllowedPathConfigurationEnum::matches`].
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum AllowedPathConfigurationEnum {
    Path(AllowedPathConfiguration),
}

impl AllowedPathConfigurationEnum {
    /// Check whether an upstream request path is allowed by this rule.
    pub fn matches(&self, path: &str, username: &str, groups: &[String]) -> bool {
        match self {
            AllowedPathConfigurationEnum::Path(config) => config.matches(path, username, groups),
        }
    }
}
