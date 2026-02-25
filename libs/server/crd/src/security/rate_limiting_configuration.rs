use crate::default::{default_disabled, default_max_requests_per_minute};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
