use common::{traits::ObjectRedis, State};
use crd::{status::ProxyKubeApiStatus, ProxyKubeApi};
use deadpool_redis::redis::cmd;
use kube::{api::PatchParams, runtime::controller::Action, Api};
use std::sync::Arc;
use tracing::{info, instrument, warn};

use crate::error::{ControllerError, Result};

#[instrument(skip(proxy, ctx), fields(name = %proxy.to_identifier()))]
pub async fn reconcile_proxy_kube_api(proxy: &ProxyKubeApi, ctx: Arc<State>) -> Result<Action> {
    const ERROR_REQUEUE_MIN_SECONDS: u64 = 5 * 60;
    const SUCCESS_REQUEUE_SECONDS: u64 = 60 * 60;

    info!("Reconciling ProxyKubeApi: {}", proxy.to_identifier());
    let id = proxy.to_identifier();
    let path = proxy.to_path();
    let retry_key = format!("requeue_retry:{}", id);
    let ps: PatchParams = PatchParams::apply("proxy-kube-api-controller").force();
    let mut proxy_cloned = proxy.clone();
    let metadata = proxy_cloned.clone().metadata;
    let ns = metadata.namespace.as_deref().unwrap_or("default");
    let name = metadata.name.as_deref().unwrap_or("unknown");
    let proxys: Api<ProxyKubeApi> = Api::namespaced(ctx.client.clone(), ns);

    // validate the proxy configuration, if it's invalid, set the status to not reachable with the error message and requeue after 5 minutes
    let mut new_status = match proxy_cloned.validate() {
        Ok(_) => ProxyKubeApiStatus::new(false, None, None),
        Err(e) => {
            tracing::error!(
                "Failed to validate ProxyKubeApi {}: {}",
                proxy.to_identifier(),
                e
            );
            ProxyKubeApiStatus::new(
                false,
                None,
                Some(format!("Failed to validate proxy configuration: {}", e)),
            )
        }
    };

    if new_status.error.is_none() {
        new_status = match proxy.clone().is_reachable(ctx.clone()).await {
            Ok(reachable) => {
                if reachable {
                    tracing::info!("ProxyKubeApi {} is reachable", proxy.to_identifier());
                    ProxyKubeApiStatus::new(true, Some(format!("/clusters/{}", path)), None)
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
                        "Failed to check if target service is reachable: {}",
                        e
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
    let mut redis_conn = ctx.get_redis_conn().await?;
    proxy_cloned.status = Some(new_status.clone());
    let proxy_json = proxy_cloned.to_json();
    match cmd("SET")
        .arg(&id)
        .arg(&proxy_json)
        .query_async::<()>(&mut redis_conn)
        .await
    {
        Ok(_) => info!("Successfully upsert ProxyKubeApi: {}", id),
        Err(err) => {
            info!("Failed to upsert ProxyKubeApi: {}. Error: {}", id, err);
        }
    }
    let requeue_action = if new_status.error.is_some() {
        let attempts = match cmd("INCR")
            .arg(&retry_key)
            .query_async::<i64>(&mut redis_conn)
            .await
        {
            Ok(value) => value.max(1) as u32,
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

        Action::requeue(std::time::Duration::from_secs(retry_delay_seconds))
    } else {
        if let Err(error) = cmd("DEL")
            .arg(&retry_key)
            .query_async::<i64>(&mut redis_conn)
            .await
        {
            warn!(
                "Failed to reset retry counter for {} ({}), continuing with success requeue",
                id, error
            );
        }
        Action::requeue(std::time::Duration::from_secs(SUCCESS_REQUEUE_SECONDS))
    };

    drop(redis_conn);

    let patch = new_status.get_patch();
    let _ = proxys
        .patch_status(name, &ps, &patch)
        .await
        .map_err(ControllerError::Kube)?;
    Ok(requeue_action)
}
