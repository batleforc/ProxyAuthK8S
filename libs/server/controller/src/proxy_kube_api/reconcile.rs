use common::agent_gateway::{agent_refused_key, count_agent};
use common::{State, traits::ObjectRedis};
use crd::service::{AgentBootstrap, Service};
use crd::status::AgentStatus;
use crd::{ProxyKubeApi, status::ProxyKubeApiStatus};
use crd_runtime::ProxyKubeApiRuntime;
use kube::{Api, runtime::controller::Action};
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tracing::{debug, info, instrument, warn};
use weebo_kit_api::status::ready;
use weebo_kit_domain::Reason as _;
use weebo_kit_domain::{Advisory, Failure};
use weebo_kit_runtime::controller::requeue_after;
use weebo_kit_runtime::metrics::ReconcileMetrics;
use weebo_kit_runtime::status::{self, StatusUpdate};

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::{FIELD_MANAGER, recorder};
use crate::reason::ReasonCode;

/// `proxyauth_reconcile_total{kind, result, reason}` and
/// `proxyauth_reconcile_duration_seconds{kind}`, on the global meter that
/// `/management/metrics` exports.
static METRICS: LazyLock<ReconcileMetrics> =
    LazyLock::new(|| ReconcileMetrics::new("proxyauthk8s", "proxyauth"));

/// At most this long between two reconciles of a `service: Agent` cluster,
/// for `status.agent` to follow its tunnels.
const AGENT_REQUEUE_SECS: u64 = 60;

/// At most this long between two reconciles of a `FrontProxy` cluster, or of
/// one preparing it (RFC 0014): its certificate and Secret follow no event.
const FRONT_PROXY_REQUEUE_SECS: u64 = 300;

/// The `kind` label of the reconcile metrics.
const KIND: &str = "ProxyKubeApi";

/// What a reconcile found, before anything is written: the `Ready` outcome,
/// the advisories next to it, and the status fields the dashboard and the
/// proxy read.
#[derive(Debug)]
pub struct Evaluation {
    pub ready: std::result::Result<String, Failure<ReasonCode>>,
    pub advisories: Vec<Advisory<ReasonCode>>,
    pub status: ProxyKubeApiStatus,
}

impl Evaluation {
    fn failed(code: ReasonCode, message: String) -> Self {
        Self {
            status: ProxyKubeApiStatus::new(false, None, Some(message.clone())),
            ready: Err(Failure::new(code, message)),
            advisories: Vec::new(),
        }
    }

    /// The reason `Ready` ends on.
    #[must_use]
    pub fn reason(&self) -> ReasonCode {
        self.ready
            .as_ref()
            .map_or_else(|failure| failure.reason, |_| ReasonCode::SUCCESS)
    }

    /// Whether the next reconcile is the steady one: after a success, or a
    /// failure only a change of the CR can clear (a change triggers a
    /// reconcile by itself).
    #[must_use]
    pub fn steady(&self) -> bool {
        match &self.ready {
            Ok(_) => true,
            Err(failure) => failure.reason == ReasonCode::InvalidSpec,
        }
    }
}

/// Validate, resolve the OIDC configuration, probe the target. Reads only:
/// nothing is cached or published here.
pub async fn evaluate(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Evaluation {
    let id = proxy.to_identifier();
    let mut evaluation = if let Err(e) = proxy.validate() {
        tracing::error!("Failed to validate ProxyKubeApi {}: {}", id, e);
        Evaluation::failed(
            ReasonCode::InvalidSpec,
            format!("Failed to validate proxy configuration: {e}"),
        )
    } else if let Some(auth_config) = &proxy.spec.auth_config
        && auth_config.oidc_provider.enabled
        && auth_config.oidc_provider.config_from.is_some()
        && let Err(e) = proxy.get_oidc_conf(ctx.clone(), false, None).await
    {
        // `validate()` deliberately skips the completeness check when
        // `config_from` is set — the fields are expected to arrive from the
        // Secret, which only a live read can confirm. Without this, a mistyped
        // Secret name is admitted, reconciles as healthy, and shows up only as
        // every request 401-ing with the cause visible in the proxy logs alone.
        tracing::error!(
            "Failed to resolve the OIDC config_from of ProxyKubeApi {}: {}",
            id,
            e
        );
        Evaluation::failed(
            ReasonCode::OidcConfigUnresolved,
            format!("Failed to resolve the OIDC configuration: {e}"),
        )
    } else if !common::config::get()
        .proxy
        .upstream_allowed_modes
        .contains(&proxy.upstream_mode())
    {
        let mode = proxy.upstream_mode().as_str();
        warn!(
            "ProxyKubeApi {} asks for upstream mode {} outside allowedModes",
            id, mode
        );
        Evaluation::failed(
            ReasonCode::UpstreamModeNotAllowed,
            format!(
                "upstream mode {mode} is not enabled on this proxy installation; ask the proxy administrator (upstreamAuth.allowedModes)"
            ),
        )
    } else if !crd::upstream_auth::SERVED_MODES.contains(&proxy.upstream_mode()) {
        let mode = proxy.upstream_mode().as_str();
        warn!(
            "ProxyKubeApi {} asks for upstream mode {}, not served by this version",
            id, mode
        );
        Evaluation::failed(
            ReasonCode::UpstreamModeUnsupported,
            format!("upstream mode {mode} is not served by this proxy version yet"),
        )
    } else {
        match proxy.clone().is_reachable(ctx.clone()).await {
            Ok(true) => {
                debug!("ProxyKubeApi {} is reachable", id);
                Evaluation {
                    ready: Ok("Target service is reachable".to_string()),
                    advisories: Vec::new(),
                    status: ProxyKubeApiStatus::new(
                        true,
                        Some(format!("/clusters/{}", proxy.to_path())),
                        None,
                    ),
                }
            }
            Ok(false) if matches!(proxy.spec.service, Service::Agent(_)) => {
                warn!("ProxyKubeApi {} has no agent tunnel", id);
                Evaluation::failed(
                    ReasonCode::AgentNoTunnel,
                    "No agent tunnel: no gateway pod holds one for this cluster".to_string(),
                )
            }
            Ok(false) => {
                warn!("ProxyKubeApi {} is not reachable", id);
                Evaluation::failed(
                    ReasonCode::TargetUnreachable,
                    "Target service is not reachable".to_string(),
                )
            }
            Err(e) => {
                tracing::error!("Failed to check if ProxyKubeApi {} is reachable: {}", id, e);
                Evaluation::failed(
                    ReasonCode::ProbeFailed,
                    format!("Failed to check if target service is reachable: {e}"),
                )
            }
        }
    };

    // Always written, `Passthrough` included: an empty `Upstream` column then
    // means a controller older than RFC 0014.
    evaluation.status.upstream_auth = Some(crd::upstream_auth::UpstreamAuthStatus {
        mode: proxy.upstream_mode(),
        ..Default::default()
    });
    super::front_proxy::evaluate(proxy, &ctx, &mut evaluation).await;
    if let Service::Agent(agent) = &proxy.spec.service {
        evaluation.status.agent = agent_status(proxy, &ctx).await;
        if let Some(status) = evaluation.status.agent.clone() {
            agent_versions(&mut evaluation, &status, &id);
        }
        let gateway = &common::config::get().agent_gateway;
        let (streams, capped) =
            agent.streams_per_tunnel(gateway.streams_per_tunnel, gateway.max_streams_per_tunnel);
        if capped {
            let message = format!(
                "limits.streams_per_tunnel is above the gateway's maximum: capped at {streams}"
            );
            warn!("ProxyKubeApi {}: {}", id, message);
            evaluation.advisories.push(Advisory {
                reason: ReasonCode::AgentLimitCapped,
                message,
            });
        }
    }
    if proxy.spec.cert.is_deprecated_insecure() {
        let message = "`cert: Insecure` is deprecated: it still verifies the target with the \
                       system trust store, rename it to `cert: SystemRoots`";
        warn!("ProxyKubeApi {}: {}", id, message);
        evaluation.advisories.push(Advisory {
            reason: ReasonCode::DeprecatedInsecureCert,
            message: message.to_string(),
        });
    }
    if let Some(security_config) = &proxy.spec.security_config {
        evaluation.status.warnings = security_config.warnings();
        for warning in &evaluation.status.warnings {
            warn!("ProxyKubeApi {}: {}", id, warning);
            evaluation.advisories.push(Advisory {
                reason: ReasonCode::RulesRestricted,
                message: warning.clone(),
            });
        }
    }
    evaluation
}

/// RFC 0013: a recent protocol refusal is the cause of `AgentNoTunnel`; an
/// agent outside the version window, or a refusal while other tunnels serve,
/// is the `AgentVersionSkew` advisory.
fn agent_versions(evaluation: &mut Evaluation, agent: &AgentStatus, id: &str) {
    let refusal = agent.refused.as_ref().map(|refused| {
        format!(
            "the gateway refused agent {} lately: it speaks protocols {}, which this proxy \
             does not; upgrade the one behind",
            refused.version, refused.protocols
        )
    });
    if agent.tunnels == 0 && evaluation.reason() == ReasonCode::AgentNoTunnel {
        if let Some(message) = refusal {
            warn!("ProxyKubeApi {}: {}", id, message);
            evaluation.status.error = Some(message.clone());
            evaluation.ready = Err(Failure::new(ReasonCode::AgentProtocolUnsupported, message));
        }
        return;
    }
    let mut messages: Vec<String> = match agent.proxy_version.as_deref().map(semver::Version::parse)
    {
        Some(Ok(proxy)) => agent
            .agents
            .iter()
            .filter_map(|row| skew(&proxy, &row.version))
            .collect(),
        _ => Vec::new(),
    };
    messages.extend(refusal);
    if !messages.is_empty() {
        let message = messages.join("; ");
        warn!("ProxyKubeApi {}: {}", id, message);
        evaluation.advisories.push(Advisory {
            reason: ReasonCode::AgentVersionSkew,
            message,
        });
    }
}

/// Why `agent` is outside the window of `proxy`, if it is: the same major,
/// a minor equal or one behind, the patch ignored (RFC 0013, decision 3).
#[must_use]
pub fn skew(proxy: &semver::Version, agent: &str) -> Option<String> {
    let Ok(parsed) = semver::Version::parse(agent) else {
        return Some(format!("agent version {agent} is not a SemVer"));
    };
    let (have, want) = ((parsed.major, parsed.minor), (proxy.major, proxy.minor));
    if have > want {
        Some(format!("agent {agent} is newer than the proxy ({proxy})"))
    } else if have.0 != want.0 || have.1.saturating_add(1) < want.1 {
        Some(format!(
            "agent {agent} is more than one minor behind the proxy ({proxy})"
        ))
    } else {
        None
    }
}

/// `status.agent` from the gateway pods' presences; `None` while Redis cannot
/// be read.
async fn agent_status(proxy: &ProxyKubeApi, ctx: &State) -> Option<AgentStatus> {
    let uid = proxy.metadata.uid.as_deref()?;
    let presences = ctx.agent_presences(uid).await.ok()?;
    let mut status = AgentStatus::default();
    for (pod, info) in presences {
        status.tunnels = status
            .tunnels
            .saturating_add(u32::try_from(info.tunnels).unwrap_or(u32::MAX));
        status.gateways.push(pod);
        status.versions.extend(info.versions);
        status.ca_fingerprints.extend(info.ca_fingerprints);
        for row in info.agents {
            count_agent(&mut status.agents, &row.version, row.protocol, row.tunnels);
        }
    }
    status.agents.sort();
    status.proxy_version = Some(env!("CARGO_PKG_VERSION").to_string());
    status.refused = ctx
        .redis_get(&agent_refused_key(uid))
        .await
        .ok()
        .flatten()
        .and_then(|record| serde_json::from_str(&record).ok());
    // Sorted: an unchanged set of tunnels writes an identical status.
    for values in [
        &mut status.gateways,
        &mut status.versions,
        &mut status.ca_fingerprints,
    ] {
        values.sort();
        values.dedup();
    }
    Some(status)
}

/// Write `proxy` with `status` to Redis, where the dashboard and the proxy
/// read it, and keep the index in sync. The copy carries the status fields
/// only: the conditions are the apiserver's (RFC 0011, decision 18).
pub async fn cache(proxy: &ProxyKubeApi, status: &ProxyKubeApiStatus, ctx: &State) -> Result<()> {
    let id = proxy.redis_key();
    // RFC 0014, decision 42: one key per cluster, under the prefix of its
    // mode. The other is dropped *first*, closed: a cluster switching to a
    // trusted mode must vanish from older binaries before it exists for the
    // new ones, never the other way round.
    let other_prefix = if proxy.redis_prefix() == crd::REDIS_PREFIX {
        crd::REDIS_TRUSTED_PREFIX
    } else {
        crd::REDIS_PREFIX
    };
    let other = format!("{other_prefix}:{}", proxy.to_path());
    ctx.delete_key(&other)
        .await
        .map_err(ControllerError::Redis)?;
    ctx.index_remove(other_prefix, &other)
        .await
        .map_err(ControllerError::Redis)?;
    let mut copy = proxy.clone();
    copy.status = Some(ProxyKubeApiStatus {
        conditions: Vec::new(),
        observed_generation: None,
        ..status.clone()
    });
    if let Err(err) = ctx.redis_set(&id, &copy.to_json(), None).await {
        // Redis is the source of truth the dashboard and proxy read from;
        // swallowing the error here would report success and drop the cluster
        // until the next reconcile. Surface it so the error policy retries.
        tracing::error!("Failed to upsert ProxyKubeApi: {}. Error: {}", id, err);
        return Err(ControllerError::Redis(err));
    }
    debug!("Successfully upsert ProxyKubeApi: {}", id);
    // Keep the index in sync so the dashboard can list clusters without a
    // `KEYS` scan, which a Redis cluster cannot answer anyway.
    if let Err(err) = ctx.index_add(proxy.redis_prefix(), &id).await {
        warn!("Failed to index ProxyKubeApi: {}. Error: {}", id, err);
    }
    Ok(())
}

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    debug!("Reconciling ProxyKubeApi: {}", proxy.to_identifier());
    let started = Instant::now();
    let mut evaluation = evaluate(proxy, ctx.clone()).await;
    if let Service::Agent(agent) = &proxy.spec.service
        && let AgentBootstrap::OneShotToken(bootstrap) = &agent.bootstrap
    {
        let message = match super::bootstrap::ensure_one_shot_token(proxy, bootstrap, &ctx).await {
            Ok(true) => None,
            Ok(false) => (evaluation.reason() == ReasonCode::AgentNoTunnel).then(|| {
                format!(
                    "No agent is connected and its OneShotToken is used or expired: annotate the \
                     ProxyKubeApi with {}=<new value> for another",
                    crd::service::REGENERATE_BOOTSTRAP_ANNOTATION
                )
            }),
            Err(err) => Some(format!("Cannot write the agent's OneShotToken: {err}")),
        };
        if let Some(message) = message {
            warn!("ProxyKubeApi {}: {}", proxy.to_identifier(), message);
            evaluation.advisories.push(Advisory {
                reason: ReasonCode::AgentBootstrapNeeded,
                message,
            });
        }
    }
    let published = publish(proxy, &evaluation, &ctx).await;
    // Counted under the code the run ended on: the evaluation's, or the write
    // that failed after it (both always carry one).
    let reason = match &published {
        Ok(()) => evaluation.reason(),
        Err(error) => error.reason().map_or(evaluation.reason(), |(code, _)| code),
    };
    METRICS.record(KIND, reason, started.elapsed());
    published?;
    // An agent's tunnels come and go without a change to the CR.
    // RFC 0014: a FrontProxy certificate expires and is renewed in place,
    // with no event on the CR; it is looked at again every few minutes.
    let front_proxy = proxy.upstream_mode() == crd::upstream_auth::UpstreamAuthMode::FrontProxy
        || evaluation
            .status
            .upstream_auth
            .as_ref()
            .is_some_and(|upstream| upstream.client_cert_secret.is_some());
    let due_in = if matches!(proxy.spec.service, Service::Agent(_)) {
        Some(AGENT_REQUEUE_SECS)
    } else {
        front_proxy.then_some(FRONT_PROXY_REQUEUE_SECS)
    };
    Ok(requeue_after(proxy, evaluation.steady(), due_in))
}

/// Cache `evaluation`, then publish it on the CR.
async fn publish(proxy: &ProxyKubeApi, evaluation: &Evaluation, ctx: &State) -> Result<()> {
    cache(proxy, &evaluation.status, ctx).await?;

    // Published on every reconcile, with no equality guard: an unchanged
    // condition keeps its `lastTransitionTime`, so a healthy CR's patch is
    // identical and the apiserver writes nothing (RFC 0011, decision 17),
    // while a repeated failure still gets its `Warning`.
    let ns = proxy.metadata.namespace.as_deref().unwrap_or("default");
    let proxys: Api<ProxyKubeApi> = Api::namespaced(ctx.client.clone(), ns);
    let previous = proxy
        .status
        .as_ref()
        .map(|status| status.conditions.clone())
        .unwrap_or_default();
    let conditions = status::publish(
        &proxys,
        recorder(ctx),
        FIELD_MANAGER,
        proxy,
        &previous,
        StatusUpdate {
            ready: &evaluation.ready,
            advisories: &evaluation.advisories,
            fields: evaluation.status.fields(),
        },
    )
    .await
    .map_err(ControllerError::Kube)?;

    let before = ready(&previous).map(|c| (&c.status, &c.reason));
    let after = ready(&conditions).map(|c| (&c.status, &c.reason));
    if before == after {
        debug!(
            "ProxyKubeApi {} unchanged: {:?}",
            proxy.to_identifier(),
            after
        );
    } else {
        info!(
            "ProxyKubeApi {} is now {:?} (was {:?})",
            proxy.to_identifier(),
            after,
            before
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crd::status::{AgentCount, AgentRefusal};

    #[test]
    fn the_version_window_is_one_minor_behind_never_newer() {
        let proxy = semver::Version::parse("0.5.2").unwrap();
        for inside in ["0.5.0", "0.5.9", "0.4.0", "0.4.7", "0.5.0-rc.1"] {
            assert_eq!(skew(&proxy, inside), None, "{inside}");
        }
        for (agent, words) in [
            ("0.3.9", "behind"),
            ("0.1.0", "behind"),
            ("0.6.0", "newer"),
            ("1.5.0", "newer"),
            ("invalid", "not a SemVer"),
        ] {
            let message = skew(&proxy, agent).expect(agent);
            assert!(
                message.contains(words) && message.contains(agent),
                "{message}"
            );
        }
        let one = semver::Version::parse("1.0.0").unwrap();
        assert!(skew(&one, "0.9.0").unwrap().contains("behind"));
    }

    fn agent(tunnels: u32, versions: &[&str], refused: bool) -> AgentStatus {
        AgentStatus {
            tunnels,
            proxy_version: Some("0.4.1".to_string()),
            agents: versions
                .iter()
                .map(|version| AgentCount {
                    version: (*version).to_string(),
                    protocol: 1,
                    tunnels: 1,
                })
                .collect(),
            refused: refused.then(|| AgentRefusal {
                reason: "protocol".to_string(),
                version: "0.7.0".to_string(),
                protocols: "2-3".to_string(),
            }),
            ..AgentStatus::default()
        }
    }

    fn no_tunnel() -> Evaluation {
        Evaluation::failed(ReasonCode::AgentNoTunnel, "No agent tunnel".to_string())
    }

    #[test]
    fn a_protocol_refusal_without_a_tunnel_names_the_cause() {
        let mut evaluation = no_tunnel();
        agent_versions(&mut evaluation, &agent(0, &[], true), "a/b");
        assert_eq!(evaluation.reason(), ReasonCode::AgentProtocolUnsupported);
        let error = evaluation.status.error.unwrap();
        assert!(error.contains("0.7.0") && error.contains("2-3"), "{error}");
        assert!(evaluation.advisories.is_empty());

        let mut evaluation = no_tunnel();
        agent_versions(&mut evaluation, &agent(0, &[], false), "a/b");
        assert_eq!(evaluation.reason(), ReasonCode::AgentNoTunnel);
    }

    #[test]
    fn a_skew_or_a_partial_refusal_is_one_advisory() {
        let served = || Evaluation {
            ready: Ok("Target service is reachable".to_string()),
            advisories: Vec::new(),
            status: ProxyKubeApiStatus::new(true, None, None),
        };
        let mut evaluation = served();
        agent_versions(
            &mut evaluation,
            &agent(2, &["0.4.0", "0.4.1"], false),
            "a/b",
        );
        assert!(evaluation.advisories.is_empty(), "{evaluation:?}");

        let mut evaluation = served();
        agent_versions(&mut evaluation, &agent(2, &["0.2.0", "0.4.1"], true), "a/b");
        assert_eq!(evaluation.reason(), ReasonCode::SUCCESS);
        let [advisory] = evaluation.advisories.as_slice() else {
            panic!("one advisory: {evaluation:?}");
        };
        assert_eq!(advisory.reason, ReasonCode::AgentVersionSkew);
        assert!(
            advisory.message.contains("0.2.0") && advisory.message.contains("2-3"),
            "{}",
            advisory.message
        );
    }

    #[test]
    fn a_success_and_an_invalid_spec_wait_for_the_steady_requeue() {
        let ok = Evaluation {
            ready: Ok("reachable".into()),
            advisories: Vec::new(),
            status: ProxyKubeApiStatus::default(),
        };
        assert!(ok.steady());
        assert!(Evaluation::failed(ReasonCode::InvalidSpec, "bad".into()).steady());
    }

    #[test]
    fn every_other_failure_backs_off() {
        for code in [
            ReasonCode::OidcConfigUnresolved,
            ReasonCode::TargetUnreachable,
            ReasonCode::ProbeFailed,
        ] {
            assert!(!Evaluation::failed(code, "down".into()).steady(), "{code}");
        }
    }

    #[test]
    fn the_metric_reason_is_the_ready_outcome() {
        let ok = Evaluation {
            ready: Ok("reachable".into()),
            advisories: Vec::new(),
            status: ProxyKubeApiStatus::default(),
        };
        assert_eq!(ok.reason(), ReasonCode::Reconciled);
        let down = Evaluation::failed(ReasonCode::ProbeFailed, "down".into());
        assert_eq!(down.reason(), ReasonCode::ProbeFailed);
    }

    #[test]
    fn a_failure_reports_its_message_as_the_status_error() {
        let failed = Evaluation::failed(ReasonCode::TargetUnreachable, "down".into());
        assert!(!failed.status.exposed);
        assert_eq!(failed.status.error.as_deref(), Some("down"));
        assert_eq!(
            failed.ready.expect_err("a failure").reason,
            ReasonCode::TargetUnreachable
        );
    }
}
