use common::{State, traits::ObjectRedis};
use crd::{ProxyKubeApi, status::ProxyKubeApiStatus};
use crd_runtime::ProxyKubeApiRuntime;
use kube::{Api, api::PatchParams, runtime::controller::Action};
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::REDIS_PREFIX;

/// Requeue delay for the first failed attempt, doubled on each consecutive one.
const ERROR_REQUEUE_MIN_SECONDS: u64 = 5 * 60;
/// Requeue delay for a healthy proxy, and the ceiling the backoff climbs to.
const SUCCESS_REQUEUE_SECONDS: u64 = 60 * 60;
/// The retry counter is only meaningful across consecutive failures; give it
/// a TTL so a cluster that recovers silently does not keep a stale count.
const RETRY_COUNTER_TTL_SECONDS: i64 = 24 * 60 * 60;

/// Exponential backoff for a proxy that keeps failing to reconcile.
///
/// Doubles [`ERROR_REQUEUE_MIN_SECONDS`] on every consecutive failure and caps
/// at [`SUCCESS_REQUEUE_SECONDS`], so a broken cluster is never polled *less*
/// often than a healthy one — the requeue is also what eventually notices the
/// cluster came back. `attempts` is the running failure count (1 on the first
/// failure); 0 is treated as 1 so a lost counter falls back to the base delay
/// rather than an empty one.
#[must_use]
pub fn retry_delay_seconds(attempts: u32) -> u64 {
    // Saturating throughout: `attempts` comes from a Redis counter, so it is
    // attacker-influenceable in the sense that a long outage can drive it
    // arbitrarily high, and an overflow here would panic the reconcile loop.
    let exponent = attempts.saturating_sub(1).min(16);
    ERROR_REQUEUE_MIN_SECONDS
        .saturating_mul(2_u64.saturating_pow(exponent))
        .min(SUCCESS_REQUEUE_SECONDS)
}

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Reconciling ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();
    let path = proxy.to_path();
    let retry_key = format!("requeue_retry:{id}");
    let ps: PatchParams = PatchParams::apply("proxy-kube-api-controller").force();
    let mut proxy_cloned = proxy.clone();
    let metadata = proxy_cloned.clone().metadata;
    let ns = metadata.namespace.as_deref().unwrap_or("default");
    let name = metadata.name.as_deref().unwrap_or("unknown");
    let proxys: Api<ProxyKubeApi> = Api::namespaced(ctx.client.clone(), ns);

    // validate the proxy configuration, if it's invalid, set the status to not reachable with the error message and requeue after 5 minutes
    let mut new_status = match proxy_cloned.validate() {
        Ok(()) => ProxyKubeApiStatus::new(false, None, None),
        Err(e) => {
            tracing::error!(
                "Failed to validate ProxyKubeApi {}: {}",
                proxy.to_identifier(),
                e
            );
            ProxyKubeApiStatus::new(
                false,
                None,
                Some(format!("Failed to validate proxy configuration: {e}")),
            )
        }
    };

    // `validate()` deliberately skips the completeness check when `config_from`
    // is set — the fields are expected to arrive from the Secret, which only a
    // live read can confirm. Without this, a mistyped Secret name is admitted,
    // reconciles as healthy, and shows up only as every request 401-ing with the
    // cause visible in the proxy logs alone.
    if new_status.error.is_none()
        && let Some(auth_config) = &proxy_cloned.spec.auth_config
        && auth_config.oidc_provider.enabled
        && auth_config.oidc_provider.config_from.is_some()
        && let Err(e) = proxy_cloned.get_oidc_conf(ctx.clone(), false, None).await
    {
        tracing::error!(
            "Failed to resolve the OIDC config_from of ProxyKubeApi {}: {}",
            proxy.to_identifier(),
            e
        );
        new_status = ProxyKubeApiStatus::new(
            false,
            None,
            Some(format!("Failed to resolve the OIDC configuration: {e}")),
        );
    }

    if new_status.error.is_none() {
        new_status = match proxy.clone().is_reachable(ctx.clone()).await {
            Ok(reachable) => {
                if reachable {
                    tracing::info!("ProxyKubeApi {} is reachable", proxy.to_identifier());
                    ProxyKubeApiStatus::new(true, Some(format!("/clusters/{path}")), None)
                } else {
                    tracing::warn!("ProxyKubeApi {} is not reachable", proxy.to_identifier());
                    ProxyKubeApiStatus::new(
                        false,
                        None,
                        Some("Target service is not reachable".to_string()),
                    )
                }
            }
            Err(e) => {
                tracing::error!(
                    "Failed to check if ProxyKubeApi {} is reachable: {}",
                    proxy.to_identifier(),
                    e
                );
                ProxyKubeApiStatus::new(
                    false,
                    None,
                    Some(format!(
                        "Failed to check if target service is reachable: {e}"
                    )),
                )
            }
        };
    }
    info!(
        "Updating status of ProxyKubeApi {}: reachable={}, error={:?}",
        proxy.to_identifier(),
        new_status.exposed,
        new_status.error
    );
    proxy_cloned.status = Some(new_status.clone());
    let proxy_json = proxy_cloned.to_json();
    match ctx.redis_set(&id, &proxy_json, None).await {
        Ok(()) => {
            info!("Successfully upsert ProxyKubeApi: {}", id);
            // Keep the index in sync so the dashboard can list clusters without
            // a `KEYS` scan, which a Redis cluster cannot answer anyway.
            if let Err(err) = ctx.index_add(REDIS_PREFIX, &id).await {
                warn!("Failed to index ProxyKubeApi: {}. Error: {}", id, err);
            }
        }
        Err(err) => {
            // Redis is the source of truth the dashboard and proxy read from;
            // swallowing the error here would report success and drop the cluster
            // for a full success-requeue interval. Surface it so the controller
            // retries with backoff via the error policy.
            tracing::error!("Failed to upsert ProxyKubeApi: {}. Error: {}", id, err);
            return Err(ControllerError::Redis(err));
        }
    }
    let requeue_action = if new_status.error.is_some() {
        let attempts = match ctx
            .incr_with_ttl(&retry_key, RETRY_COUNTER_TTL_SECONDS)
            .await
        {
            Ok(value) => u32::try_from(value.max(1)).unwrap_or(u32::MAX),
            Err(error) => {
                warn!(
                    "Failed to increment retry counter for {} ({}), using base retry delay",
                    id, error
                );
                1
            }
        };

        let retry_delay_seconds = retry_delay_seconds(attempts);
        info!(
            "Requeueing ProxyKubeApi {} after error, attempt {}, retrying in {} seconds due to error: {:?}",
            id, attempts, retry_delay_seconds, new_status.error
        );

        Action::requeue(std::time::Duration::from_secs(retry_delay_seconds))
    } else {
        if let Err(error) = ctx.delete_key(&retry_key).await {
            warn!(
                "Failed to reset retry counter for {} ({}), continuing with success requeue",
                id, error
            );
        }
        info!(
            "ProxyKubeApi {} is healthy, requeueing after {} seconds",
            id, SUCCESS_REQUEUE_SECONDS
        );
        Action::requeue(std::time::Duration::from_secs(SUCCESS_REQUEUE_SECONDS))
    };

    // patch only if necessary to avoid unnecessary API calls and potential conflicts
    let current_status = proxy.status.as_ref();
    if !current_status.is_some_and(|status| status.equal(&new_status)) {
        info!(
            "Patching status of ProxyKubeApi {}: reachable={}, error={:?}",
            proxy.to_identifier(),
            new_status.exposed,
            new_status.error
        );
        let patch = new_status.patch();
        let _ = proxys
            .patch_status(name, &ps, &patch)
            .await
            .map_err(ControllerError::Kube)?;
    }
    info!(
        "Finished reconciling ProxyKubeApi: {}",
        proxy.to_identifier()
    );
    Ok(requeue_action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_failure_waits_the_base_delay() {
        // The counter is 1 on the first failure. A 0 means the counter was lost
        // (Redis down, TTL expired mid-flight) — same base delay, never zero.
        assert_eq!(retry_delay_seconds(0), ERROR_REQUEUE_MIN_SECONDS);
        assert_eq!(retry_delay_seconds(1), ERROR_REQUEUE_MIN_SECONDS);
        assert_eq!(retry_delay_seconds(1), 300);
    }

    #[test]
    fn each_consecutive_failure_doubles_the_delay() {
        assert_eq!(retry_delay_seconds(2), 600);
        assert_eq!(retry_delay_seconds(3), 1_200);
        assert_eq!(retry_delay_seconds(4), 2_400);
    }

    #[test]
    fn the_delay_is_capped_at_the_success_interval() {
        // 300 * 2^4 = 4800 would exceed the hourly success requeue: a failing
        // cluster must not be polled less often than a healthy one, or a
        // recovery could go unnoticed for longer than a plain refresh.
        assert_eq!(retry_delay_seconds(5), SUCCESS_REQUEUE_SECONDS);
        assert_eq!(retry_delay_seconds(5), 3_600);
        for attempts in [6, 20, 1_000, u32::MAX] {
            assert_eq!(
                retry_delay_seconds(attempts),
                SUCCESS_REQUEUE_SECONDS,
                "attempt {attempts} should stay at the cap"
            );
        }
    }

    #[test]
    fn the_delay_never_decreases_and_never_overflows() {
        // The exponent is clamped before the shift, so no input panics in debug
        // or wraps in release — a Redis counter can climb without bound during
        // a long outage.
        let mut previous = 0;
        for attempts in 0..64 {
            let delay = retry_delay_seconds(attempts);
            assert!(
                delay >= previous,
                "attempt {attempts} went backwards: {delay} < {previous}"
            );
            assert!((ERROR_REQUEUE_MIN_SECONDS..=SUCCESS_REQUEUE_SECONDS).contains(&delay));
            previous = delay;
        }
        assert_eq!(retry_delay_seconds(u32::MAX), SUCCESS_REQUEUE_SECONDS);
    }
}
