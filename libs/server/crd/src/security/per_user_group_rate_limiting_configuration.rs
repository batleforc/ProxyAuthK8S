use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-user group rate limiting configuration
///
/// Groups come from the user resolved by the configured authentication, so no
/// claim mapping is needed here.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct PerUserGroupRateLimitingConfiguration {
    /// Group name
    pub group: String,
    /// The maximum number of requests per minute for this group
    /// This setting overrides the global rate limiting setting
    /// 0 disables the rate limiting for this group
    pub max_requests_per_minute: u32,
}
