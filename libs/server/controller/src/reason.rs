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
        /// A `service: Agent` cluster with no gateway pod holding its tunnel
        /// (RFC 0012).
        AgentNoTunnel,
        /// Advisory: `limits.streams_per_tunnel` is above the gateway's
        /// maximum and was capped (RFC 0012, decision 27).
        AgentLimitCapped,
        /// Advisory: no agent is connected and no usable `OneShotToken` waits
        /// for one, or it could not be written (RFC 0012 §4.3).
        AgentBootstrapNeeded,
        /// Advisory: an agent's version is outside the window, more than one
        /// minor behind the proxy or newer, or not a SemVer (RFC 0013).
        AgentVersionSkew,
        /// No tunnel, and the gateway refused the agent's protocols lately
        /// (RFC 0013): the cause of `AgentNoTunnel`.
        AgentProtocolUnsupported,
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
            | Self::Deleted
            | Self::AgentLimitCapped
            | Self::AgentBootstrapNeeded
            | Self::AgentVersionSkew => Severity::Advisory,
            Self::InvalidSpec
            | Self::OidcConfigUnresolved
            | Self::TargetUnreachable
            | Self::ProbeFailed
            | Self::CacheWriteFailed
            | Self::StatusWriteFailed
            | Self::FinalizerFailed
            | Self::AgentNoTunnel
            | Self::AgentProtocolUnsupported => Severity::Blocking,
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
    fn the_catalog_has_the_codes_of_rfc_0011_0012_and_0013() {
        assert_eq!(ReasonCode::ALL.len(), 16);
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
                &ReasonCode::AgentLimitCapped,
                &ReasonCode::AgentBootstrapNeeded,
                &ReasonCode::AgentVersionSkew,
            ]
        );
    }
}
