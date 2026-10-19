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

/// Name of the Lease the replicas compete for, in `State::lease_namespace`.
pub const LEASE_NAME: &str = "proxy-auth-k8s-leader-election";
/// How long a leader keeps the Lease without renewing it.
pub const LEASE_TTL: Duration = Duration::from_secs(15);
/// How often the Lease is renewed (or retried): well under [`LEASE_TTL`], so
/// a live leader never lets it lapse.
pub const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);

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

/// The Lease `holder_id` competes for as [`LEASE_NAME`] in `namespace`, held
/// for `ttl` without renewal.
#[must_use]
pub fn lease_lock(
    client: kube::Client,
    namespace: &str,
    holder_id: &str,
    ttl: Duration,
) -> LeaseLock {
    LeaseLock::new(
        client,
        namespace,
        LeaseLockParams {
            holder_id: holder_id.to_string(),
            lease_name: LEASE_NAME.into(),
            lease_ttl: ttl,
        },
    )
}

/// Keep `state.is_leader` in step with `leadership`, renewing (or trying to
/// acquire) it every `renew_every`. Never returns.
pub async fn run_leader_election(state: State, leadership: LeaseLock, renew_every: Duration) {
    let mut interval = interval(renew_every);
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

    let leadership = lease_lock(
        state.client.clone(),
        &state.lease_namespace,
        &state.lease_name,
        LEASE_TTL,
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
        () = run_leader_election(state.clone(), leadership, LEASE_RENEW_INTERVAL) => {
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
