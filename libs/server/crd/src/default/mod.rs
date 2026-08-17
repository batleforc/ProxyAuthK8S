use crate::authentication_configuration::ValidateAgainst;

#[must_use]
pub fn default_enabled() -> bool {
    true
}
#[must_use]
pub fn default_disabled() -> bool {
    false
}

#[must_use]
pub fn default_max_failed_logins() -> u32 {
    5
}

#[must_use]
pub fn default_ban_duration() -> u32 {
    300
}

#[must_use]
pub fn default_max_requests_per_minute() -> u32 {
    60
}

#[must_use]
pub fn default_empty_array<T>() -> Vec<T> {
    Vec::new()
}

#[must_use]
pub fn default_empty_string() -> String {
    String::new()
}

/// Default token-validation backend when a `ProxyKubeApi` does not set one.
///
/// This is intentionally always `Kubernetes` (a `SelfSubjectReview` against the
/// target apiserver): it is the fail-closed choice, valid for every cluster
/// regardless of whether an OIDC provider is configured. Operators who want the
/// token validated against their OIDC provider must set `validate_against`
/// explicitly.
#[must_use]
pub fn default_validate_against() -> ValidateAgainst {
    ValidateAgainst::Kubernetes
}
