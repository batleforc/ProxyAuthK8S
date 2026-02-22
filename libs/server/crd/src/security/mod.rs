use crate::default::{
    default_ban_duration, default_disabled, default_empty_array, default_enabled,
    default_max_failed_logins, default_max_requests_per_minute,
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
    #[serde(default = "default_empty_array::<PerUserGroupRateLimitingConfiguration>")]
    pub per_user_group_rate_limiting: Vec<PerUserGroupRateLimitingConfiguration>,
    /// Allowed resources, limit the access to the proxy to only these resources, if empty all resources are allowed
    pub allowed_ressources: Vec<AllowedPathConfigurationEnum>,
}

impl Default for SecurityConfiguration {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            fail2login_equal_ban: Fail2LoginEqualBanConfiguration::default(),
            per_user_group_rate_limiting: default_empty_array(),
            allowed_ressources: default_empty_array(),
        }
    }
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

/// Enum of the allowed paths configuration, currently only supports path and crd, but can be extended in the future
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum AllowedPathConfigurationEnum {
    Path(AllowedPathConfiguration),
    Crd(AllowedCrdConfiguration),
}

/// Allowed path configuration, used in conjunction with the allowed_paths configuration
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AllowedPathConfiguration {
    /// The path to allow, if the request path starts with this path, it will be allowed
    pub path: String,

    /// Wether or not the ressource is namespaced, if true, the namespace access rules will be applied to this resource
    pub namespace: NamespacedAccessConfiguration,
    /// Namespaced or not
    /// default: true
    #[serde(default = "default_enabled")]
    pub namespaced: bool,
}

/// Allowed crd configuration, used in conjunction with the allowed_paths configuration
/// /apis/{group}/{version}/namespaces/{namespace}/{kind}/
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AllowedCrdConfiguration {
    /// The group of the crd
    pub group: String,
    /// The version of the crd
    pub version: String,
    /// The kind of the crd
    pub kind: String,
    /// Wether or not the kind has a specific plural form, if true, the plural form will be used in the path instead of the kind
    /// for example, if the kind is "MyResource" and the plural form is "MyResources"
    /// the path will be /apis/{group}/{version}/namespaces/{namespace}/myresources/
    /// instead of       /apis/{group}/{version}/namespaces/{namespace}/myresource/
    /// In case of cluster-scoped crd
    /// the path will be /apis/{group}/{version}/myresources/
    /// instead of       /apis/{group}/{version}/myresource/
    pub plural: Option<String>,

    /// Wether or not the ressource is namespaced, if true, the namespace access rules will be applied to this resource
    pub namespace: NamespacedAccessConfiguration,
    /// Namespaced or not
    /// default: true
    #[serde(default = "default_enabled")]
    pub namespaced: bool,
}

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

/// Namespace rule kind, either :
/// - A vec of allowed namespaces, if the user is only allowed to access a specific set of namespaces
/// - A vec of denied namespaces, if the user is allowed to access all namespaces except a specific set of namespaces
/// - A parametised rule, if the user is allowed to access namespaces that match a certain pattern, for example namespaces that start with "dev-{username}"
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum NamespacedAccessRuleKind {
    AllowedNamespaces(Vec<String>),
    DeniedNamespaces(Vec<String>),
    ParametisedRule(String),
}
