use crate::default::{
    default_ban_duration, default_disabled, default_enabled, default_max_failed_logins,
    default_max_requests_per_minute,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SecurityConfiguration {
    /// Whether the token is validated beforehand
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Configuration for banning users after multiple failed login attempts
    pub fail2login_equal_ban: Fail2LoginEqualBanConfiguration,
    /// Per group rate limiting configuration
    /// This take precedence over the global rate limiting configuration
    pub per_user_group_rate_limiting: Vec<PerUserGroupRateLimitingConfiguration>,
}

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

/// Rate limiting configuration
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RateLimitingConfiguration {
    /// If the feature is enabled
    /// default: false
    #[serde(default = "default_disabled")]
    pub enabled: bool,
    /// The maximum number of requests per minute
    /// default: 60
    /// 0 disables the rate global rate limiting
    #[serde(default = "default_max_requests_per_minute")]
    pub max_requests_per_minute: u32,
}

/// Per-user group rate limiting configuration
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct PerUserGroupRateLimitingConfiguration {
    /// Group name
    pub group: String,
    /// The maximum number of requests per minute for this group
    /// This setting overrides the global rate limiting setting
    /// 0 disables the rate limiting for this group
    pub max_requests_per_minute: u32,
    /// Claim to identify the user group, used in conjunction with claim_mappings in the JWTAuthenticator
    pub claim: String,
}
