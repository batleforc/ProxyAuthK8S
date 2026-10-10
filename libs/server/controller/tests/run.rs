//! `controller::run` and the leader election loop when the apiserver cannot
//! be reached. Needs neither Redis nor a cluster: the Kubernetes client
//! points at a port nothing listens on, and nothing here touches Redis.
//!
//! The live paths (taking the Lease, losing it, handing it over) run against
//! a real apiserver in the `api` crate's envtest tier
//! (`envtest_leader_election.rs`, `envtest_controller.rs`).

mod support;

use std::{sync::atomic::Ordering, time::Duration};

use controller::{error::ControllerError, lease_lock, run, run_leader_election};
use support::{UNREACHABLE_REDIS_URL, state_with_redis};

#[tokio::test]
async fn run_stops_when_the_crd_cannot_be_listed() {
    let state = state_with_redis(UNREACHABLE_REDIS_URL);
    let result = tokio::time::timeout(Duration::from_secs(30), run(state))
        .await
        .expect("run must fail fast, not hang, when the apiserver is unreachable");
    assert!(
        matches!(result, Err(ControllerError::CrdUnavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_leader_that_cannot_renew_steps_down() {
    let state = state_with_redis(UNREACHABLE_REDIS_URL);
    // A leader whose apiserver goes away: renewing fails with an error, not
    // with `NotAcquired`, and must still clear the flag.
    state.is_leader.store(true, Ordering::Relaxed);
    let lease = lease_lock(
        state.client.clone(),
        "default",
        "replica-a",
        Duration::from_secs(2),
    );
    let election = tokio::spawn(run_leader_election(
        state.clone(),
        lease,
        Duration::from_millis(50),
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while state.is_leader.load(Ordering::Relaxed) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a leader that cannot reach the apiserver must step down"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    election.abort();
}
