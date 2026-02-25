use crate::default::{default_ban_duration, default_disabled, default_max_failed_logins};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Pseudo Fail2Ban configuration
/// If the user has too many failed login attempts, he will be banned for a certain time
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Fail2LoginEqualBanConfiguration {
    /// If the feature is enabled
    /// default: false
    #[serde(default = "default_disabled")]
    pub enabled: bool,
    /// The number of failed login attempts before the user is banned
    /// default: 5
    #[serde(default = "default_max_failed_logins")]
    pub max_failed_logins: u32,
    /// The duration of the ban in seconds
    /// default: 300 (5 minutes)
    /// 0 means permanent ban
    #[serde(default = "default_ban_duration")]
    pub ban_duration: u32,
    /// If the time is exponentially increased with each failed login attempt
    /// default: false
    #[serde(default = "default_disabled")]
    pub exponential_backoff: bool,
}

impl Default for Fail2LoginEqualBanConfiguration {
    fn default() -> Self {
        Self {
            enabled: default_disabled(),
            max_failed_logins: default_max_failed_logins(),
            ban_duration: default_ban_duration(),
            exponential_backoff: default_disabled(),
        }
    }
}
