//! The controller's view of a `FrontProxy` cluster (RFC 0014 §4.3, §6.4):
//! its certificate's Secret, the certificate itself, the audience mode. In
//! the effective mode a problem is `Ready=False` (the proxy then answers
//! `503`); while a `Passthrough` cluster prepares the switch it is an
//! advisory, `FrontProxyNotReady`.

use common::State;
use common::token_audience::AudienceValidationMode;
use crd::ProxyKubeApi;
use crd::upstream_auth::{UpstreamAuthMode, UpstreamAuthStatus, front_proxy};
use weebo_kit_domain::{Advisory, Failure};

use super::reconcile::Evaluation;
use crate::reason::ReasonCode;

/// What the controller reads off the certificate.
#[derive(Debug, PartialEq, Eq)]
pub struct CertFacts {
    pub not_before: i64,
    pub not_after: i64,
    pub subject: String,
    /// A CN or an O starting with `system:` (T7).
    pub reserved: bool,
}

/// The first certificate of `pem`.
///
/// # Errors
///
/// Why it cannot be read, without its content.
pub fn parse_cert(pem: &str) -> Result<CertFacts, String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| "tls.crt is not a PEM certificate".to_string())?;
    let cert = pem
        .parse_x509()
        .map_err(|_| "tls.crt is not an X.509 certificate".to_string())?;
    let subject = cert.subject();
    let reserved = subject
        .iter_common_name()
        .chain(subject.iter_organization())
        .filter_map(|attribute| attribute.as_str().ok())
        .any(|value| value.to_ascii_lowercase().starts_with("system:"));
    Ok(CertFacts {
        not_before: cert.validity().not_before.timestamp(),
        not_after: cert.validity().not_after.timestamp(),
        subject: subject.to_string(),
        reserved,
    })
}

/// Whether `facts` expire within `min(30 days, a third of the lifetime)` of
/// `now`: a 24 h certificate renewed by cert-manager does not warn all day.
#[must_use]
pub fn expiring(facts: &CertFacts, now: i64) -> bool {
    let lifetime = facts.not_after.saturating_sub(facts.not_before).max(0);
    let window = (30 * 24 * 3600_i64).min(lifetime / 3);
    facts.not_after.saturating_sub(now) < window
}

/// A cluster still in `Passthrough` with both prefixes set and no
/// `issuer_key`: it prepares `FrontProxy`.
fn preparing(proxy: &ProxyKubeApi) -> bool {
    proxy.upstream_mode() == UpstreamAuthMode::Passthrough
        && proxy.spec.upstream_auth.as_ref().is_some_and(|upstream| {
            upstream.issuer_key.is_none()
                && upstream
                    .user_prefix
                    .as_deref()
                    .is_some_and(|p| !p.is_empty())
                && upstream
                    .group_prefix
                    .as_deref()
                    .is_some_and(|p| !p.is_empty())
        })
}

/// In the effective mode `Ready=False` with `code`, the cluster no longer
/// exposed; while preparing, the `FrontProxyNotReady` advisory.
fn refuse(evaluation: &mut Evaluation, effective: bool, code: ReasonCode, message: String) {
    if effective {
        evaluation.status.exposed = false;
        evaluation.status.error = Some(message.clone());
        evaluation.ready = Err(Failure::new(code, message));
    } else {
        evaluation.advisories.push(Advisory {
            reason: ReasonCode::FrontProxyNotReady,
            message,
        });
    }
}

/// The `FrontProxy` checks of `proxy`, folded into `evaluation`.
pub async fn evaluate(proxy: &ProxyKubeApi, ctx: &State, evaluation: &mut Evaluation) {
    let effective = proxy.upstream_mode() == UpstreamAuthMode::FrontProxy;
    if !effective && !preparing(proxy) {
        return;
    }
    // Only an effective failure overrides a Ready that already failed.
    if effective && evaluation.ready.is_err() {
        return;
    }
    let mut status = UpstreamAuthStatus {
        mode: proxy.upstream_mode(),
        client_cert_target: Some(front_proxy::canonical_target(proxy)),
        ..UpstreamAuthStatus::default()
    };
    match front_proxy::secret_name(proxy) {
        Ok(name) => status.client_cert_secret = Some(format!("{}/{name}", ctx.lease_namespace)),
        Err(e) => {
            evaluation.status.upstream_auth = Some(status);
            refuse(
                evaluation,
                effective,
                ReasonCode::ClientCertNameTooLong,
                e.to_string(),
            );
            return;
        }
    }
    if effective && AudienceValidationMode::from_env() != AudienceValidationMode::Enforce {
        evaluation.status.upstream_auth = Some(status);
        refuse(
            evaluation,
            true,
            ReasonCode::AudienceValidationRequired,
            "upstream mode FrontProxy requires OIDC audience validation; ask the proxy administrator (OIDC_AUDIENCE_VALIDATION=enforce)".to_string(),
        );
        return;
    }
    let loaded = front_proxy::load(ctx.client.clone(), &ctx.lease_namespace, proxy).await;
    let (cert, _key) = match loaded {
        Ok(pair) => pair,
        // Nothing to report while preparing: the Secret is simply not there yet.
        Err(front_proxy::FrontProxyCertError::Missing { .. }) if !effective => {
            evaluation.status.upstream_auth = Some(status);
            return;
        }
        Err(e) => {
            let code = match e.reason() {
                "ClientCertTargetMismatch" => ReasonCode::ClientCertTargetMismatch,
                "ClientCertNameTooLong" => ReasonCode::ClientCertNameTooLong,
                _ => ReasonCode::ClientCertMissing,
            };
            evaluation.status.upstream_auth = Some(status);
            refuse(evaluation, effective, code, e.to_string());
            return;
        }
    };
    match parse_cert(&cert) {
        Ok(facts) => {
            status.client_cert_subject = Some(facts.subject.clone());
            status.client_cert_not_after =
                k8s_openapi::jiff::Timestamp::from_second(facts.not_after)
                    .ok()
                    .map(|t| t.to_string());
            let now = k8s_openapi::jiff::Timestamp::now().as_second();
            if facts.reserved {
                refuse(
                    evaluation,
                    effective,
                    ReasonCode::ClientCertSubjectReserved,
                    format!(
                        "the FrontProxy certificate's subject {} starts with system: use a dedicated certificate",
                        facts.subject
                    ),
                );
            } else if facts.not_after <= now {
                refuse(
                    evaluation,
                    effective,
                    ReasonCode::ClientCertMissing,
                    "the FrontProxy certificate has expired; renew it in its Secret".to_string(),
                );
            } else if expiring(&facts, now) {
                evaluation.advisories.push(Advisory {
                    reason: ReasonCode::ClientCertExpiring,
                    message: format!(
                        "the FrontProxy certificate expires at {}",
                        status.client_cert_not_after.clone().unwrap_or_default()
                    ),
                });
            }
        }
        Err(message) => refuse(
            evaluation,
            effective,
            ReasonCode::ClientCertMissing,
            message,
        ),
    }
    evaluation.status.upstream_auth = Some(status);
    // The trust probe's last word (RFC 0014 §5.4, decision 45), once the
    // certificate itself is sound.
    if effective && evaluation.ready.is_err() {
        return;
    }
    match common::upstream_trust::get(&proxy.to_path()) {
        Some(common::upstream_trust::Trust::NotHonoured(seen)) => refuse(
            evaluation,
            effective,
            ReasonCode::UpstreamIdentityNotHonoured,
            format!(
                "the apiserver sees {seen} instead of the identity the proxy asserts: check --requestheader-client-ca-file (never --client-ca-file) and the --requestheader-*-headers flags"
            ),
        ),
        Some(common::upstream_trust::Trust::Rejected) => refuse(
            evaluation,
            effective,
            ReasonCode::FrontProxyNotTrusted,
            "the apiserver refuses the proxy's certificate (401): check that its CA is in --requestheader-client-ca-file and its CN in --requestheader-allowed-names".to_string(),
        ),
        Some(common::upstream_trust::Trust::Unknown(why)) => evaluation.advisories.push(Advisory {
            reason: ReasonCode::UpstreamTrustUnknown,
            message: format!("the trust probe reached no conclusion: {why}"),
        }),
        Some(common::upstream_trust::Trust::Trusted) | None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(subject: &str, days: i64) -> String {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
        let mut name = rcgen::DistinguishedName::new();
        for part in subject.split(',') {
            let (key, value) = part.split_once('=').expect("key=value");
            let kind = if key == "O" {
                rcgen::DnType::OrganizationName
            } else {
                rcgen::DnType::CommonName
            };
            name.push(kind, value);
        }
        params.distinguished_name = name;
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now.saturating_sub(time::Duration::days(1));
        params.not_after = now.saturating_add(time::Duration::days(days));
        let key = rcgen::KeyPair::generate().expect("key");
        params.self_signed(&key).expect("cert").pem()
    }

    #[test]
    fn the_subject_and_validity_are_read_and_a_system_subject_is_flagged() {
        let facts = parse_cert(&cert("CN=proxyauth-team-a-prod", 90)).expect("parsed");
        assert!(facts.subject.contains("proxyauth-team-a-prod"));
        assert!(!facts.reserved);
        assert!(
            parse_cert(&cert("CN=proxy,O=system:masters", 90))
                .unwrap()
                .reserved
        );
        assert!(parse_cert(&cert("CN=System:admin", 90)).unwrap().reserved);
        assert!(parse_cert("not a pem").is_err());
    }

    /// A 24 h certificate does not warn all day; a 90-day one warns its last
    /// 30 days.
    #[test]
    fn expiring_is_relative_to_the_lifetime() {
        let day = 24 * 3600;
        let short = CertFacts {
            not_before: 0,
            not_after: day,
            subject: String::new(),
            reserved: false,
        };
        assert!(!expiring(&short, day / 2));
        assert!(expiring(&short, day - 3600 * 4));
        let long = CertFacts {
            not_before: 0,
            not_after: 90 * day,
            ..short
        };
        assert!(!expiring(&long, 50 * day));
        assert!(expiring(&long, 61 * day));
    }
}
