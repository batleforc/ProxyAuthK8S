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
use tracing::info;

use crate::proxy_kube_api::{error_policy_proxy_kube_api, main_reconcile_proxy_kube_api};

pub mod error;
pub mod proxy_kube_api;

pub async fn run_leader_election(state: State, leadership: LeaseLock) {
    let mut interval = interval(Duration::from_secs(5));
    loop {
        match leadership.try_acquire_or_renew().await {
            Ok(lease) => {
                let acquired = matches!(lease, LeaseLockResult::Acquired(_));
                // The election must demote as well as promote: `try_acquire_or_renew`
                // returns `Ok(NotAcquired)` (not an `Err`) when another instance holds
                // the lease, so a leader that fails to renew in time would keep
                // `is_leader = true` and run a second, competing reconcile loop
                // (split-brain) unless we clear it here.
                if state.is_leader.load(Ordering::Relaxed) != acquired {
                    if acquired {
                        info!("Successfully acquired leadership");
                    } else {
                        info!("Lost leadership, stepping down as leader");
                    }
                    state.is_leader.store(acquired, Ordering::Relaxed);
                }
            }
            Err(e) => {
                tracing::error!("Not the leader: {}", e);
                state.is_leader.store(false, Ordering::Relaxed);
            }
        }
        interval.tick().await;
    }
}

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

    let leadership = LeaseLock::new(
        state.client.clone(),
        &state.lease_namespace.clone(),
        LeaseLockParams {
            holder_id: state.lease_name.clone(),
            lease_name: "proxy-auth-k8s-leader-election".into(),
            lease_ttl: Duration::from_secs(15),
        },
    );

    match leadership.try_acquire_or_renew().await {
        Ok(lease) => {
            let acquired = matches!(lease, LeaseLockResult::Acquired(_));
            state.is_leader.store(acquired, Ordering::Relaxed);
            if acquired {
                info!("Successfully acquired leadership (initial)");
            } else {
                info!("Another instance holds the lease, starting as non-leader");
            }
        }
        Err(e) => {
            tracing::error!("Initial leader election attempt failed: {}", e);
        }
    }

    let controller_state = Arc::new(state.clone());

    tokio::select! {
        _ = Controller::new(proxy_kube_apis.clone(), Config::default().any_semantic())
            .shutdown_on_signal()
            .run(
                main_reconcile_proxy_kube_api,
                error_policy_proxy_kube_api,
                controller_state.clone(),
            )
            .filter_map(|x| async move { std::result::Result::ok(x) })
            .for_each(|_| futures::future::ready(())) => {
        },
        _ = run_leader_election(state.clone(), leadership) => {
        }
    };
}
