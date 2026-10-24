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
use crate::proxy_kube_api::{FIELD_MANAGER, REDIS_PREFIX, recorder};
use crate::reason::ReasonCode;

/// `proxyauth_reconcile_total{kind, result, reason}` and
/// `proxyauth_reconcile_duration_seconds{kind}`, on the global meter that
/// `/management/metrics` exports.
static METRICS: LazyLock<ReconcileMetrics> =
    LazyLock::new(|| ReconcileMetrics::new("proxyauthk8s", "proxyauth"));

/// At most this long between two reconciles of a `service: Agent` cluster,
/// for `status.agent` to follow its tunnels.
const AGENT_REQUEUE_SECS: u64 = 60;

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

    if let Service::Agent(agent) = &proxy.spec.service {
        evaluation.status.agent = agent_status(proxy, &ctx).await;
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
    }
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
    let id = proxy.to_identifier();
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
    if let Err(err) = ctx.index_add(REDIS_PREFIX, &id).await {
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
    let due_in = matches!(proxy.spec.service, Service::Agent(_)).then_some(AGENT_REQUEUE_SECS);
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
