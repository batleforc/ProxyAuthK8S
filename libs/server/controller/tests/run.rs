//! `controller::run` and the leader election loop when the apiserver cannot
//! be reached. Needs neither Redis nor a cluster: the Kubernetes client
//! points at a port nothing listens on, and nothing here touches Redis.
//!
//! The live paths (taking the Lease, losing it, handing it over) run against
//! a real apiserver in the `api` crate's envtest tier
//! (`envtest_leader_election.rs`, `envtest_controller.rs`).

#[expect(dead_code, reason = "this binary uses only part of the shared harness")]
mod support;

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

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
async fn a_replica_that_cannot_reach_the_apiserver_does_not_lead() {
    let state = state_with_redis(UNREACHABLE_REDIS_URL);
    // A stale flag (it cannot happen through the loop): the election clears
    // it before competing, and never sets it without holding the Lease.
    state.is_leader.store(true, Ordering::Relaxed);
    let lease = Arc::new(lease_lock(
        state.client.clone(),
        "default",
        "replica-a",
        Duration::from_secs(2),
    ));
    let election = tokio::spawn(run_leader_election(
        state.clone(),
        lease,
        Duration::from_millis(50),
        Duration::from_secs(2),
    ));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while state.is_leader.load(Ordering::Relaxed) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a replica that cannot reach the apiserver must not lead"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // And it stays a follower while the apiserver is unreachable.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!state.is_leader.load(Ordering::Relaxed));
    election.abort();
}
