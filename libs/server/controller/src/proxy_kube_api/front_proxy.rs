//! The controller's view of a `FrontProxy` cluster (RFC 0014 §4.3, §6.4):
//! its certificate's Secret, the certificate itself, the audience mode. In
//! the effective mode a problem is `Ready=False` (the proxy then answers
//! `503`); while a `Passthrough` cluster prepares the switch it is an
//! advisory, `FrontProxyNotReady`. Requeued every minute, the trust probe's
//! round.

use common::State;
use common::token_audience::AudienceValidationMode;
use crd::ProxyKubeApi;
use crd::upstream_auth::{UpstreamAuthMode, UpstreamAuthStatus, UpstreamTrust, front_proxy};
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
    refuse_as(
        evaluation,
        effective,
        code,
        ReasonCode::FrontProxyNotReady,
        message,
    );
}

/// [`refuse`] with the advisory of the mode being prepared.
pub(super) fn refuse_as(
    evaluation: &mut Evaluation,
    effective: bool,
    code: ReasonCode,
    preparing: ReasonCode,
    message: String,
) {
    if effective {
        evaluation.status.exposed = false;
        evaluation.status.error = Some(message.clone());
        evaluation.ready = Err(Failure::new(code, message));
    } else {
        evaluation.advisories.push(Advisory {
            reason: preparing,
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
    let verdict = record_trust(proxy, &mut status);
    evaluation.status.upstream_auth = Some(status);
    // The trust probe's last word (RFC 0014 §5.4, decision 45), once the
    // certificate itself is sound.
    if effective && evaluation.ready.is_err() {
        return;
    }
    judge_trust(
        verdict,
        evaluation,
        effective,
        ReasonCode::FrontProxyNotReady,
        (
            ReasonCode::FrontProxyNotTrusted,
            "the apiserver refuses the proxy's certificate (401): check that its CA is in --requestheader-client-ca-file and its CN in --requestheader-allowed-names".to_string(),
        ),
    );
}

/// The trust probe's last verdict on `proxy`, written into `status`
/// (decision 60).
pub(super) fn record_trust(
    proxy: &ProxyKubeApi,
    status: &mut UpstreamAuthStatus,
) -> Option<common::upstream_trust::Trust> {
    let (trust, since) = common::upstream_trust::get(&proxy.to_path())?;
    status.trust = Some(match &trust {
        common::upstream_trust::Trust::Trusted => UpstreamTrust::Trusted,
        common::upstream_trust::Trust::NotHonoured(_) => UpstreamTrust::NotHonoured,
        common::upstream_trust::Trust::Rejected => UpstreamTrust::Rejected,
        common::upstream_trust::Trust::Unknown(_) => UpstreamTrust::Unknown,
        common::upstream_trust::Trust::Unguarded => UpstreamTrust::Unguarded,
    });
    status.trust_since = k8s_openapi::jiff::Timestamp::try_from(since)
        .ok()
        .map(|t| t.to_string());
    Some(trust)
}

/// What a verdict does to `Ready` (decision 45): a broken trust closes an
/// effective cluster and warns a preparing one (`preparing`); `rejected` is
/// the mode's reason and message for a `401`.
pub(super) fn judge_trust(
    verdict: Option<common::upstream_trust::Trust>,
    evaluation: &mut Evaluation,
    effective: bool,
    preparing: ReasonCode,
    rejected: (ReasonCode, String),
) {
    match verdict {
        Some(common::upstream_trust::Trust::NotHonoured(seen)) => refuse_as(
            evaluation,
            effective,
            ReasonCode::UpstreamIdentityNotHonoured,
            preparing,
            format!(
                "the apiserver sees {seen} instead of the identity the proxy asserts: check how it authenticates the proxy (--requestheader-* flags, or the issuer's claimMappings)"
            ),
        ),
        Some(common::upstream_trust::Trust::Rejected) => {
            refuse_as(evaluation, effective, rejected.0, preparing, rejected.1);
        }
        Some(common::upstream_trust::Trust::Unguarded) => refuse_as(
            evaluation,
            effective,
            ReasonCode::IssuerNotTrusted,
            preparing,
            "the apiserver accepts an unprefixed identity from this issuer: add the userValidationRules of the issuer guide".to_string(),
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

    // The certificate Secret against a wiremock apiserver (RFC 0015 P3):
    // every verdict `evaluate` draws from what it reads.
    mod against_an_apiserver {
        use super::*;
        use crd::upstream_auth::UpstreamAuth;
        use crd::upstream_auth::front_proxy::{
            MANAGED_BY_LABEL, MANAGED_BY_VALUE, TARGET_ANNOTATION,
        };
        use serde_json::json;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        const SECRET: &str =
            "/api/v1/namespaces/default/secrets/proxyauth-frontproxy-default.fronting";
        const TARGET: &str = "external:https://k:6443";

        fn state(kube: &MockServer) -> State {
            drop(rustls::crypto::ring::default_provider().install_default());
            let config = kube::Config::new(kube.uri().parse().unwrap());
            State::from_parts(
                kube::Client::try_from(config).unwrap(),
                common::kv_memory::InMemoryKv::new(),
                common::oidc_conf::OidcConf {
                    client_id: "proxyauthk8s".to_string(),
                    client_secret: None,
                    issuer_url: "http://127.0.0.1:1".to_string(),
                    scopes: "openid".to_string(),
                    audience: "proxyauthk8s".to_string(),
                    accept_authorized_party: false,
                    redirect_url: None,
                },
                "https://proxy.example.com".to_string(),
                "https://front.example.com".to_string(),
            )
        }

        fn fronting(mode: UpstreamAuthMode) -> ProxyKubeApi {
            let mut proxy = ProxyKubeApi::new(
                "fronting",
                serde_json::from_value(json!({
                    "cert": { "SystemRoots": true },
                    "service": { "ExternalService": { "url": "https://k:6443" } },
                }))
                .unwrap(),
            );
            proxy.metadata.namespace = Some("default".into());
            proxy.spec.upstream_auth = Some(UpstreamAuth {
                mode,
                user_prefix: Some("proxyauth:".into()),
                group_prefix: Some("proxyauth:".into()),
                issuer_key: None,
            });
            proxy
        }

        /// A certificate valid from `from` days ago to `to` days ahead.
        fn cert_between(subject: &str, from: i64, to: i64) -> String {
            let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
            let mut name = rcgen::DistinguishedName::new();
            name.push(rcgen::DnType::CommonName, subject);
            params.distinguished_name = name;
            let now = time::OffsetDateTime::now_utc();
            params.not_before = now.saturating_sub(time::Duration::days(from));
            params.not_after = now.saturating_add(time::Duration::days(to));
            let key = rcgen::KeyPair::generate().unwrap();
            params.self_signed(&key).unwrap().pem()
        }

        async fn serve(kube: &MockServer, target: &str, cert: &str) {
            let secret = json!({
                "apiVersion": "v1", "kind": "Secret",
                "metadata": {
                    "name": "proxyauth-frontproxy-default.fronting", "namespace": "default",
                    "labels": { MANAGED_BY_LABEL: MANAGED_BY_VALUE },
                    "annotations": { TARGET_ANNOTATION: target },
                },
                "data": {
                    "tls.crt": base64_encode(cert),
                    "tls.key": base64_encode("KEY"),
                },
            });
            Mock::given(method("GET"))
                .and(path(SECRET))
                .respond_with(ResponseTemplate::new(200).set_body_json(secret))
                .mount(kube)
                .await;
        }

        fn base64_encode(value: &str) -> String {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(value)
        }

        async fn evaluated(kube: &MockServer, mode: UpstreamAuthMode) -> Evaluation {
            let mut evaluation = Evaluation {
                ready: Ok("ok".into()),
                advisories: Vec::new(),
                status: crd::status::ProxyKubeApiStatus::default(),
            };
            evaluate(&fronting(mode), &state(kube), &mut evaluation).await;
            evaluation
        }

        fn refusal(evaluation: Evaluation) -> ReasonCode {
            evaluation.ready.expect_err("not Ready").reason
        }

        #[tokio::test]
        async fn a_sound_certificate_for_this_target_leaves_the_cluster_ready() {
            let kube = MockServer::start().await;
            serve(&kube, TARGET, &cert_between("proxyauth-fronting", 1, 90)).await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::FrontProxy).await;
            assert!(evaluation.advisories.is_empty());
            assert!(evaluation.ready.is_ok());
            let status = evaluation.status.upstream_auth.expect("status");
            assert_eq!(status.client_cert_target.as_deref(), Some(TARGET));
            assert_eq!(
                status.client_cert_secret.as_deref(),
                Some("default/proxyauth-frontproxy-default.fronting")
            );
            assert!(
                status
                    .client_cert_subject
                    .unwrap()
                    .contains("proxyauth-fronting")
            );
            assert!(status.client_cert_not_after.is_some());
        }

        /// Decision 56: a certificate issued for another target is never
        /// presented to this one.
        #[tokio::test]
        async fn a_certificate_for_another_target_is_refused() {
            let kube = MockServer::start().await;
            serve(
                &kube,
                "external:https://other:6443",
                &cert_between("p", 1, 90),
            )
            .await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::FrontProxy).await;
            assert_eq!(refusal(evaluation), ReasonCode::ClientCertTargetMismatch);
        }

        /// T2: a `system:` subject would be an apiserver identity of its own.
        #[tokio::test]
        async fn a_system_subject_is_refused() {
            let kube = MockServer::start().await;
            serve(&kube, TARGET, &cert_between("system:kube-proxy", 1, 90)).await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::FrontProxy).await;
            assert_eq!(refusal(evaluation), ReasonCode::ClientCertSubjectReserved);
        }

        #[tokio::test]
        async fn an_expired_certificate_is_refused_an_expiring_one_warned() {
            let kube = MockServer::start().await;
            serve(&kube, TARGET, &cert_between("p", 2, 0)).await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::FrontProxy).await;
            assert_eq!(refusal(evaluation), ReasonCode::ClientCertMissing);

            let kube = MockServer::start().await;
            serve(&kube, TARGET, &cert_between("p", 80, 10)).await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::FrontProxy).await;
            assert!(evaluation.ready.is_ok());
            let reasons: Vec<_> = evaluation.advisories.iter().map(|a| a.reason).collect();
            assert_eq!(reasons, [ReasonCode::ClientCertExpiring]);
        }

        /// No Secret: refused in the effective mode, silence while preparing
        /// (it is simply not there yet).
        #[tokio::test]
        async fn a_missing_secret_is_refused_only_in_the_effective_mode() {
            let kube = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(SECRET))
                .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                    "kind": "Status", "apiVersion": "v1", "status": "Failure",
                    "reason": "NotFound", "code": 404,
                })))
                .mount(&kube)
                .await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::FrontProxy).await;
            assert_eq!(refusal(evaluation), ReasonCode::ClientCertMissing);
            let preparing = evaluated(&kube, UpstreamAuthMode::Passthrough).await;
            assert!(preparing.ready.is_ok());
            assert!(preparing.advisories.is_empty());
        }

        /// While preparing, a bad certificate is an advisory, never a
        /// `Ready=False` of a cluster that still works in `Passthrough`.
        #[tokio::test]
        async fn a_bad_certificate_while_preparing_is_an_advisory() {
            let kube = MockServer::start().await;
            serve(
                &kube,
                "external:https://other:6443",
                &cert_between("p", 1, 90),
            )
            .await;
            let evaluation = evaluated(&kube, UpstreamAuthMode::Passthrough).await;
            assert!(evaluation.ready.is_ok());
            let reasons: Vec<_> = evaluation.advisories.iter().map(|a| a.reason).collect();
            assert_eq!(reasons, [ReasonCode::FrontProxyNotReady]);
        }
    }
}
