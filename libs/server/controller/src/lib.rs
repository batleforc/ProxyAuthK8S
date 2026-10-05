//! Kubernetes controller for the `ProxyKubeApi` custom resource.
//!
//! Runs the reconcile loop that watches `ProxyKubeApi` objects, mirrors their
//! desired state into Redis for the request path to consume, and manages
//! leader election so only one replica reconciles at a time.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use common::State;
use crd::ProxyKubeApi;
use futures_util::StreamExt;
use kube::{
    Api,
    runtime::{Controller, watcher::Config},
};
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};
use tokio::time::interval;
use tracing::info;

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::{error_policy_proxy_kube_api, main_reconcile_proxy_kube_api};

pub mod error;
pub mod proxy_kube_api;

/// Decide whether a lease renewal outcome changes this instance's leadership.
///
/// Returns `Some(new_value)` when `is_leader` must be updated (promotion when
/// the lease was acquired, demotion when it was not), `None` when the state is
/// unchanged. Demotion matters as much as promotion: `try_acquire_or_renew`
/// reports a lost lease as `Ok(NotAcquired)`, not as an error.
#[must_use]
pub fn leadership_transition(currently_leader: bool, acquired: bool) -> Option<bool> {
    (currently_leader != acquired).then_some(acquired)
}

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
                if let Some(is_leader) =
                    leadership_transition(state.is_leader.load(Ordering::Relaxed), acquired)
                {
                    if is_leader {
                        info!("Successfully acquired leadership");
                    } else {
                        info!("Lost leadership, stepping down as leader");
                    }
                    state.is_leader.store(is_leader, Ordering::Relaxed);
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

/// Run the `ProxyKubeApi` controller and the leader election loop.
///
/// # Errors
///
/// Returns [`ControllerError::CrdUnavailable`] when the `ProxyKubeApi`
/// resources cannot be listed at startup (CRD not installed, missing RBAC,
/// apiserver unreachable). The caller is expected to stop the process so the
/// pod restarts (and surfaces as `CrashLoopBackOff`) rather than serving with
/// a controller that will never fill Redis.
pub async fn run(state: State) -> Result<()> {
    let client = state.client.clone();
    let proxy_kube_apis = Api::<ProxyKubeApi>::all(client.clone());
    if let Err(e) = proxy_kube_apis.list(&Default::default()).await {
        let err = ControllerError::CrdUnavailable(e);
        tracing::error!(error = %err, "Controller cannot start");
        return Err(err);
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
        () = Controller::new(proxy_kube_apis.clone(), Config::default().any_semantic())
            .shutdown_on_signal()
            .run(
                main_reconcile_proxy_kube_api,
                error_policy_proxy_kube_api,
                controller_state.clone(),
            )
            .filter_map(|x| async move { std::result::Result::ok(x) })
            .for_each(|_| futures_util::future::ready(())) => {
        },
        () = run_leader_election(state.clone(), leadership) => {
        }
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::leadership_transition;

    #[test]
    fn follower_that_acquires_the_lease_is_promoted() {
        assert_eq!(leadership_transition(false, true), Some(true));
    }

    #[test]
    fn leader_that_fails_to_renew_is_demoted() {
        // Guards against split-brain: a `NotAcquired` renewal must clear the flag.
        assert_eq!(leadership_transition(true, false), Some(false));
    }

    #[test]
    fn renewing_leader_is_left_unchanged() {
        assert_eq!(leadership_transition(true, true), None);
    }

    #[test]
    fn follower_that_still_lacks_the_lease_is_left_unchanged() {
        assert_eq!(leadership_transition(false, false), None);
    }
}
