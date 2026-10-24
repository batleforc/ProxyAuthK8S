//! The reason codes a `ProxyKubeApi` condition, Event or reconcile metric
//! carries (RFC 0011 §4.2): a closed catalog, so a cause can be filtered,
//! counted and documented, never an ad-hoc string.

use weebo_kit_domain::{Reason, Severity};

weebo_kit_domain::reason_codes! {
    pub enum ReasonCode {
        /// `Ready: True`: the target answers, the cluster is served.
        Reconciled,
        /// `validate()` refuses the spec.
        InvalidSpec,
        /// The `oidc_provider.config_from` Secret cannot be read.
        OidcConfigUnresolved,
        /// The probe got an answer, and it says the target is not usable.
        TargetUnreachable,
        /// The probe itself failed.
        ProbeFailed,
        /// Writing (or deleting) the Redis copy failed.
        CacheWriteFailed,
        /// The status patch failed after `status::publish`'s retries.
        StatusWriteFailed,
        /// Adding or removing the finalizer failed.
        FinalizerFailed,
        /// Advisory: rules whose meaning a port, verb or exec restriction changes.
        RulesRestricted,
        /// Advisory: `cert: Insecure`, the deprecated name of `SystemRoots`.
        DeprecatedInsecureCert,
        /// Advisory: the finalizer removed the cluster from Redis.
        Deleted,
    }
}

impl Reason for ReasonCode {
    const SUCCESS: Self = Self::Reconciled;

    fn as_str(self) -> &'static str {
        ReasonCode::as_str(self)
    }

    fn severity(self) -> Severity {
        match self {
            Self::Reconciled
            | Self::RulesRestricted
            | Self::DeprecatedInsecureCert
            | Self::Deleted => Severity::Advisory,
            Self::InvalidSpec
            | Self::OidcConfigUnresolved
            | Self::TargetUnreachable
            | Self::ProbeFailed
            | Self::CacheWriteFailed
            | Self::StatusWriteFailed
            | Self::FinalizerFailed => Severity::Blocking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_a_valid_condition_reason() {
        weebo_kit_domain::reason::assert_pascal_case(ReasonCode::ALL);
    }

    #[test]
    fn the_catalog_has_the_rfc_s_eleven_codes() {
        assert_eq!(ReasonCode::ALL.len(), 11);
    }

    #[test]
    fn only_the_success_and_the_advisories_do_not_block() {
        let advisory: Vec<_> = ReasonCode::ALL
            .iter()
            .filter(|code| code.severity() == Severity::Advisory)
            .collect();
        assert_eq!(
            advisory,
            [
                &ReasonCode::Reconciled,
                &ReasonCode::RulesRestricted,
                &ReasonCode::DeprecatedInsecureCert,
                &ReasonCode::Deleted,
            ]
        );
    }
}
