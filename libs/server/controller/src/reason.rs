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
        /// The cluster asks for an upstream mode outside
        /// `upstreamAuth.allowedModes` (RFC 0014).
        UpstreamModeNotAllowed,
        /// The cluster asks for an upstream mode this version does not serve
        /// yet (RFC 0014, decision 34).
        UpstreamModeUnsupported,
        /// `FrontProxy`: its Secret is missing, unreadable or unlabelled
        /// (RFC 0014 §4.3).
        ClientCertMissing,
        /// `FrontProxy`: the Secret was issued for another target (T29).
        ClientCertTargetMismatch,
        /// `FrontProxy`: `proxyauth-frontproxy-<ns>.<name>` is too long.
        ClientCertNameTooLong,
        /// `FrontProxy`: the certificate's CN or O starts with `system:`.
        ClientCertSubjectReserved,
        /// A trusted mode with `OIDC_AUDIENCE_VALIDATION` other than
        /// `enforce` (decision 50).
        AudienceValidationRequired,
        /// Advisory: the `FrontProxy` certificate expires within
        /// `min(30 days, a third of its lifetime)`.
        ClientCertExpiring,
        /// Advisory: a cluster preparing `FrontProxy` (still `Passthrough`)
        /// would not be served once switched; the message says why.
        FrontProxyNotReady,
        /// The trust probe saw another identity than the one asserted
        /// (RFC 0014 §5.4): the certificate taken for itself, or a header
        /// the apiserver does not read.
        UpstreamIdentityNotHonoured,
        /// The trust probe got `401`: the CA or `allowed-names`.
        FrontProxyNotTrusted,
        /// Advisory: the trust probe reached no conclusion (transport,
        /// `5xx`, no agent); the cluster stays served.
        UpstreamTrustUnknown,
        /// `Issuer`: the key Secret is unreadable, not the proxy's, or
        /// cannot be written; the message names the Secret.
        IssuerKeyUnavailable,
        /// `Issuer`: the path of `issuer_key.issuer_url` is not
        /// `/issuer/<ns>/<name>`, the route the proxy serves.
        IssuerUrlMismatch,
        /// `Issuer`: the trust probe got `401`, or the apiserver also took
        /// an unprefixed identity (no `userValidationRules`).
        IssuerNotTrusted,
        /// Advisory: a cluster preparing `Issuer` (still `Passthrough` with
        /// `issuer_key`) would not be served once switched; the message
        /// says why.
        IssuerNotReady,
        /// Advisory: the key has not rotated for `rotation_days` + 1.
        IssuerKeyRotationOverdue,
        /// Advisory: `Issuer` behind an agent: the apiserver must still
        /// reach the proxy's public URL for the JWKS (decision 13).
        IssuerBehindAgent,
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
            | Self::AgentVersionSkew
            | Self::ClientCertExpiring
            | Self::FrontProxyNotReady
            | Self::UpstreamTrustUnknown
            | Self::IssuerNotReady
            | Self::IssuerKeyRotationOverdue
            | Self::IssuerBehindAgent => Severity::Advisory,
            Self::InvalidSpec
            | Self::OidcConfigUnresolved
            | Self::TargetUnreachable
            | Self::ProbeFailed
            | Self::CacheWriteFailed
            | Self::StatusWriteFailed
            | Self::FinalizerFailed
            | Self::AgentNoTunnel
            | Self::AgentProtocolUnsupported
            | Self::UpstreamModeNotAllowed
            | Self::UpstreamModeUnsupported
            | Self::ClientCertMissing
            | Self::ClientCertTargetMismatch
            | Self::ClientCertNameTooLong
            | Self::ClientCertSubjectReserved
            | Self::AudienceValidationRequired
            | Self::UpstreamIdentityNotHonoured
            | Self::FrontProxyNotTrusted
            | Self::IssuerKeyUnavailable
            | Self::IssuerUrlMismatch
            | Self::IssuerNotTrusted => Severity::Blocking,
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
    fn the_catalog_has_the_codes_of_rfc_0011_to_0014() {
        assert_eq!(ReasonCode::ALL.len(), 34);
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
                &ReasonCode::ClientCertExpiring,
                &ReasonCode::FrontProxyNotReady,
                &ReasonCode::UpstreamTrustUnknown,
                &ReasonCode::IssuerNotReady,
                &ReasonCode::IssuerKeyRotationOverdue,
                &ReasonCode::IssuerBehindAgent,
            ]
        );
    }
}
