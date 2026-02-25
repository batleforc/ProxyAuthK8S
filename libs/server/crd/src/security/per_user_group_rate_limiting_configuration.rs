use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-user group rate limiting configuration
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct PerUserGroupRateLimitingConfiguration {
    /// Group name
    pub group: String,
    /// The maximum number of requests per minute for this group
    /// This setting overrides the global rate limiting setting
    /// 0 disables the rate limiting for this group
    pub max_requests_per_minute: u32,
    /// Claim to identify the user group, used in conjunction with claim_mappings in the JWTAuthenticator or the generated user from either kube or oauth2 authentication
    pub claim: String,
}
