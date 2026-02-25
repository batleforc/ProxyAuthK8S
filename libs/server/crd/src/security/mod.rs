use crate::default::{default_empty_array, default_enabled};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod allowed_crd_configuration;
mod allowed_path_configuration;
mod allowed_path_configuration_enum;
mod fail2login_equal_ban_configuration;
mod namespaced_access_configuration;
mod namespaced_access_rule_kind;
mod per_user_group_rate_limiting_configuration;
mod rate_limiting_configuration;

pub use allowed_crd_configuration::AllowedCrdConfiguration;
pub use allowed_path_configuration::AllowedPathConfiguration;
pub use allowed_path_configuration_enum::AllowedPathConfigurationEnum;
pub use fail2login_equal_ban_configuration::Fail2LoginEqualBanConfiguration;
pub use namespaced_access_configuration::NamespacedAccessConfiguration;
pub use namespaced_access_rule_kind::NamespacedAccessRuleKind;
pub use per_user_group_rate_limiting_configuration::PerUserGroupRateLimitingConfiguration;
pub use rate_limiting_configuration::RateLimitingConfiguration;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SecurityConfiguration {
    /// Whether the token is validated beforehand
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    // TODO /// Configuration for banning users after multiple failed login attempts
    // pub fail2login_equal_ban: Fail2LoginEqualBanConfiguration,
    // TODO /// Per group rate limiting configuration
    // /// This take precedence over the global rate limiting configuration
    // #[serde(default = "default_empty_array::<PerUserGroupRateLimitingConfiguration>")]
    // pub per_user_group_rate_limiting: Vec<PerUserGroupRateLimitingConfiguration>,
    /// Allowed resources, limit the access to the proxy to only these resources, if empty all resources are allowed
    pub allowed_ressources: Vec<AllowedPathConfigurationEnum>,
}

impl Default for SecurityConfiguration {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            // fail2login_equal_ban: Fail2LoginEqualBanConfiguration::default(),
            // per_user_group_rate_limiting: default_empty_array(),
            allowed_ressources: default_empty_array(),
        }
    }
}

impl SecurityConfiguration {
    pub fn validate(&self) -> Result<(), String> {
        for allowed_resource in &self.allowed_ressources {
            match allowed_resource {
                AllowedPathConfigurationEnum::Path(config) => {
                    config.validate()?;
                } // TODO : AllowedPathConfigurationEnum::AllowedCrdConfiguration(config) => {
                  //     // TODO : validate the crd configuration, for example, check if the group, version and kind are valid --- IGNORE ---
                  //     let _ = config;
                  // }
            }
        }
        Ok(())
    }
}
