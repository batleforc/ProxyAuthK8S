use common::{traits::ObjectRedis, State};
use crd::{status::ProxyKubeApiStatus, ProxyKubeApi};
use crd_runtime::ProxyKubeApiRuntime;
use kube::{api::PatchParams, runtime::controller::Action, Api};
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::REDIS_PREFIX;

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    const ERROR_REQUEUE_MIN_SECONDS: u64 = 5 * 60;
    const SUCCESS_REQUEUE_SECONDS: u64 = 60 * 60;
    // The retry counter is only meaningful across consecutive failures; give it
    // a TTL so a cluster that recovers silently does not keep a stale count.
    const RETRY_COUNTER_TTL_SECONDS: i64 = 24 * 60 * 60;

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

        let exponent = attempts.saturating_sub(1).min(16);
        let retry_delay_seconds = ERROR_REQUEUE_MIN_SECONDS
            .saturating_mul(2_u64.saturating_pow(exponent))
            .min(SUCCESS_REQUEUE_SECONDS);
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
