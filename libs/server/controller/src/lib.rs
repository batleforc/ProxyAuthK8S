use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use common::State;
use crd::ProxyKubeApi;
use futures::StreamExt;
use kube::{
    runtime::{watcher::Config, Controller},
    Api,
};
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};
use tokio::time::interval;

use crate::proxy_kube_api::{error_policy_proxy_kube_api, main_reconcile_proxy_kube_api};

pub mod error;
pub mod proxy_kube_api;

pub async fn run(state: State) {
    let client = state.client.clone();
    let proxy_kube_apis = Api::<ProxyKubeApi>::all(client.clone());
    if let Err(e) = proxy_kube_apis.list(&Default::default()).await {
        tracing::error!(
            "Failed to list ProxyKubeApi resources (the CRD maybe not installed) : {}",
            e
        );
        panic!("Failed to list ProxyKubeApi resources: {}", e);
    }
    let controller_state = Arc::new(state.clone());

    let leadership = LeaseLock::new(
        client,
        &state.lease_namespace.clone(),
        LeaseLockParams {
            holder_id: state.lease_name.clone(),
            lease_name: "proxy-auth-k8s-leader-election".into(),
            lease_ttl: Duration::from_secs(15),
        },
    );
    let mut interval = interval(Duration::from_secs(5));
    loop {
        let mut kill = false;
        tokio::select! {
            _ = Controller::new(proxy_kube_apis.clone(), Config::default().any_semantic())
            .shutdown_on_signal()
            .run(
                main_reconcile_proxy_kube_api,
                error_policy_proxy_kube_api,
                controller_state.clone(),
            )
            .filter_map(|x| async move { std::result::Result::ok(x) })
            .for_each(|_| futures::future::ready(()))
            => {
              kill = true;
            },
            _ = interval.tick() => {
                match leadership.try_acquire_or_renew().await {
                    Ok(lease) => {
                        tracing::info!("This instance is now the leader");
                        state.is_leader.store(matches!(lease, LeaseLockResult::Acquired(_)), Ordering::Relaxed)
                    },
                    Err(e) => {
                        tracing::error!("Not the leader: {}", e);
                        state.is_leader.store(false, Ordering::Relaxed);
                    }
                }
            }
        };
        if kill {
            break;
        }
    }
}
