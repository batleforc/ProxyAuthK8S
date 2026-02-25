use crate::authentication_configuration::validate_against::ValidateAgainst;

pub fn default_enabled() -> bool {
    true
}
pub fn default_disabled() -> bool {
    false
}

pub fn default_max_failed_logins() -> u32 {
    5
}

pub fn default_ban_duration() -> u32 {
    300
}

pub fn default_max_requests_per_minute() -> u32 {
    60
}

pub fn default_empty_array<T>() -> Vec<T> {
    Vec::new()
}

pub fn default_empty_string() -> String {
    String::new()
}

pub fn default_validate_against() -> ValidateAgainst {
    ValidateAgainst::Kubernetes
}
