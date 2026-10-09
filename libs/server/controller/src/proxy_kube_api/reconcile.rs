use common::{State, traits::ObjectRedis};
use crd::{ProxyKubeApi, status::ProxyKubeApiStatus};
use crd_runtime::ProxyKubeApiRuntime;
use kube::{Api, api::PatchParams, runtime::controller::Action};
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::REDIS_PREFIX;

/// Base requeue delay after a failed reconcile (first consecutive failure).
pub const ERROR_REQUEUE_MIN_SECONDS: u64 = 5 * 60;
/// Requeue delay after a healthy reconcile; also the cap of the error backoff.
pub const SUCCESS_REQUEUE_SECONDS: u64 = 60 * 60;
/// The retry counter is only meaningful across consecutive failures; give it
/// a TTL so a cluster that recovers silently does not keep a stale count.
const RETRY_COUNTER_TTL_SECONDS: i64 = 24 * 60 * 60;
/// Exponent cap: keeps `2^exponent` far from overflow (the delay is capped at
/// [`SUCCESS_REQUEUE_SECONDS`] long before this matters anyway).
const MAX_BACKOFF_EXPONENT: u32 = 16;

/// Convert the raw Redis retry counter into a 1-based attempt number.
///
/// A counter of `0` (should not happen after `INCR`, but is tolerated) counts
/// as the first attempt; values beyond `u32::MAX` saturate.
#[must_use]
pub fn attempts_from_counter(counter: u64) -> u32 {
    u32::try_from(counter.max(1)).unwrap_or(u32::MAX)
}

/// Exponential-backoff requeue delay, in seconds, for the `attempts`-th
/// consecutive failed reconcile.
///
/// Doubles from [`ERROR_REQUEUE_MIN_SECONDS`] on each extra failure and is
/// capped at [`SUCCESS_REQUEUE_SECONDS`], so a broken cluster is never retried
/// less often than a healthy one is re-checked.
#[must_use]
pub fn retry_delay_seconds(attempts: u32) -> u64 {
    let exponent = attempts.saturating_sub(1).min(MAX_BACKOFF_EXPONENT);
    ERROR_REQUEUE_MIN_SECONDS
        .saturating_mul(2_u64.saturating_pow(exponent))
        .min(SUCCESS_REQUEUE_SECONDS)
}

/// Redis key of the consecutive-failure counter driving the backoff for the
/// proxy cached under `id` (see [`ProxyKubeApi::to_identifier`]).
#[must_use]
pub fn retry_counter_key(id: &str) -> String {
    format!("requeue_retry:{id}")
}

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    info!("Reconciling ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();
    let path = proxy.to_path();
    let retry_key = retry_counter_key(&id);
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
            Ok(value) => attempts_from_counter(value),
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
    fn first_failure_uses_the_base_delay() {
        assert_eq!(retry_delay_seconds(1), ERROR_REQUEUE_MIN_SECONDS);
        assert_eq!(retry_delay_seconds(1), 300);
    }

    #[test]
    fn zero_attempts_is_treated_as_the_first_failure() {
        assert_eq!(retry_delay_seconds(0), ERROR_REQUEUE_MIN_SECONDS);
    }

    #[test]
    fn delay_doubles_on_each_consecutive_failure() {
        assert_eq!(retry_delay_seconds(2), 600);
        assert_eq!(retry_delay_seconds(3), 1200);
        assert_eq!(retry_delay_seconds(4), 2400);
    }

    #[test]
    fn delay_is_capped_at_the_success_requeue_interval() {
        // 300 * 2^4 = 4800 > 3600: the fifth failure is the first one capped.
        assert_eq!(retry_delay_seconds(5), SUCCESS_REQUEUE_SECONDS);
        assert_eq!(retry_delay_seconds(17), SUCCESS_REQUEUE_SECONDS);
        assert_eq!(retry_delay_seconds(1_000), SUCCESS_REQUEUE_SECONDS);
    }

    #[test]
    fn delay_never_overflows_for_huge_attempt_counts() {
        assert_eq!(retry_delay_seconds(u32::MAX), SUCCESS_REQUEUE_SECONDS);
    }

    #[test]
    fn delay_is_monotonic_and_never_below_the_base() {
        let mut previous = 0;
        for attempts in 0..64 {
            let delay = retry_delay_seconds(attempts);
            assert!(delay >= previous, "delay decreased at attempt {attempts}");
            assert!(delay >= ERROR_REQUEUE_MIN_SECONDS);
            assert!(delay <= SUCCESS_REQUEUE_SECONDS);
            previous = delay;
        }
    }

    #[test]
    fn counter_is_converted_to_a_one_based_attempt() {
        assert_eq!(attempts_from_counter(0), 1);
        assert_eq!(attempts_from_counter(1), 1);
        assert_eq!(attempts_from_counter(7), 7);
    }

    #[test]
    fn oversized_counter_saturates_instead_of_wrapping() {
        assert_eq!(attempts_from_counter(u64::from(u32::MAX) + 1), u32::MAX);
        assert_eq!(attempts_from_counter(u64::MAX), u32::MAX);
        assert_eq!(
            retry_delay_seconds(attempts_from_counter(u64::MAX)),
            SUCCESS_REQUEUE_SECONDS
        );
    }
}
