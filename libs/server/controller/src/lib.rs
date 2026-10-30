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
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{
    Api,
    runtime::{Controller, watcher::Config},
};
use tracing::{info, warn};
use weebo_kit_runtime::controller::drive;
use weebo_kit_runtime::leader::{self, LeaseLock};

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::{error_policy_proxy_kube_api, main_reconcile_proxy_kube_api};

pub mod error;
pub mod proxy_kube_api;
pub mod reason;

/// Name of the Lease the replicas compete for, in `State::lease_namespace`.
pub const LEASE_NAME: &str = "proxy-auth-k8s-leader-election";
/// How long a leader keeps the Lease without renewing it.
pub const LEASE_TTL: Duration = Duration::from_secs(15);
/// How often the Lease is renewed (or retried): well under [`LEASE_TTL`], so
/// a live leader never lets it lapse.
pub const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(5);

/// The Lease `holder_id` competes for as [`LEASE_NAME`] in `namespace`, held
/// for `ttl` without renewal: the kit's `LeaseLock`, whose writes carry the
/// `resourceVersion` they were computed from (a conflict means "not leader")
/// and which judges expiry on this replica's monotonic clock (RFC 0011, T7).
#[must_use]
pub fn lease_lock(
    client: kube::Client,
    namespace: &str,
    holder_id: &str,
    ttl: Duration,
) -> LeaseLock {
    leader::lease(client, namespace, LEASE_NAME, holder_id.to_string(), ttl)
}

/// Keep `state.is_leader` in step with `lease`, forever: acquire it (retried
/// every `renew_every`), lead while `hold` renews it, and step down as soon as
/// `hold` says it may be lost — taken over, or not renewed within `ttl` —
/// then compete again. Never exits the process: every replica also serves the
/// proxy (RFC 0011, decision 28).
pub async fn run_leader_election(
    state: State,
    lease: Arc<LeaseLock>,
    renew_every: Duration,
    ttl: Duration,
) {
    loop {
        state.is_leader.store(false, Ordering::Relaxed);
        leader::acquire(&lease, renew_every).await;
        info!("Successfully acquired leadership");
        state.is_leader.store(true, Ordering::Relaxed);
        let why = leader::hold(&lease, renew_every, ttl).await;
        state.is_leader.store(false, Ordering::Relaxed);
        warn!(reason = %why, "Lost leadership, stepping down as leader");
    }
}

/// The `ProxyKubeApi` CRD's name.
const PROXY_KUBE_API_CRD: &str = "proxykubeapis.weebo.si.rs";

/// Whether the CRD's served `v1` schema knows `spec.upstream_auth`.
fn crd_has_upstream_auth(crd: &CustomResourceDefinition) -> bool {
    crd.spec.versions.iter().any(|version| {
        version.name == "v1"
            && version
                .schema
                .as_ref()
                .and_then(|schema| schema.open_api_v3_schema.as_ref())
                .and_then(|root| root.properties.as_ref()?.get("spec"))
                .and_then(|spec| spec.properties.as_ref())
                .is_some_and(|spec| spec.contains_key("upstream_auth"))
    })
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
    // RFC 0014, decision 42: a CRD older than the binary prunes
    // `upstream_auth` on read, and a trusted cluster would be cached as
    // Passthrough. Closed: no reconcile on such a CRD.
    let crds = Api::<CustomResourceDefinition>::all(client.clone());
    match crds.get(PROXY_KUBE_API_CRD).await {
        Ok(crd) if crd_has_upstream_auth(&crd) => {}
        Ok(_) => {
            let err = ControllerError::CrdOutdated;
            tracing::error!(error = %err, "Controller cannot start");
            return Err(err);
        }
        Err(e) => {
            let err = ControllerError::CrdUnavailable(e);
            tracing::error!(error = %err, "Controller cannot start");
            return Err(err);
        }
    }

    let leadership = Arc::new(lease_lock(
        state.client.clone(),
        &state.lease_namespace,
        &state.lease_name,
        LEASE_TTL,
    ));
    // `proxyauth_leader`: 1 on the replica holding the Lease (RFC 0011,
    // decision 35). Kept alive by this binding.
    let is_leader = state.is_leader.clone();
    let _leader = opentelemetry::global::meter("proxyauthk8s")
        .u64_observable_gauge("proxyauth_leader")
        .with_description("1 on the replica holding the leader Lease")
        .with_callback(move |gauge| {
            gauge.observe(u64::from(is_leader.load(Ordering::Relaxed)), &[])
        })
        .build();

    let controller_state = Arc::new(state.clone());
    // `proxyauth_controller_watch_failing`: 1 while the watch has kept failing
    // past the kit's threshold (5 min), else 0. Kept alive by this binding.
    let health = state.controller_health.clone();
    let _watch_failing = opentelemetry::global::meter("proxyauthk8s")
        .u64_observable_gauge("proxyauth_controller_watch_failing")
        .with_description("1 while the ProxyKubeApi watch keeps failing")
        .with_callback(move |gauge| gauge.observe(u64::from(health.live().is_err()), &[]))
        .build();

    tokio::select! {
        // `drive` logs every failure (they were dropped here before) and
        // reports watch errors to `controller_health`; reconcile failures
        // already show on the CR as conditions and Events.
        () = drive(
            "ProxyKubeApi",
            state.controller_health.clone(),
            Controller::new(proxy_kube_apis.clone(), Config::default().any_semantic())
                .shutdown_on_signal()
                .run(
                    main_reconcile_proxy_kube_api,
                    error_policy_proxy_kube_api,
                    controller_state.clone(),
                ),
        ) => {
        },
        () = run_leader_election(state.clone(), leadership.clone(), LEASE_RENEW_INTERVAL, LEASE_TTL) => {
        }
        // RFC 0014 §5.3: the Issuer keys of clusters that are gone.
        () = proxy_kube_api::issuer::run_sweep(controller_state.clone()) => {
        }
    };
    // Shutting down: hand the Lease over now instead of letting the next
    // leader wait out its TTL (decision 36).
    if state.is_leader.swap(false, Ordering::Relaxed) {
        match leadership.step_down().await {
            Ok(()) => info!("Released the leader Lease"),
            Err(error) => warn!(%error, "Could not release the leader Lease"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crd_from(manifest: &str) -> CustomResourceDefinition {
        serde_yaml_ng::from_str(manifest).expect("crd")
    }

    #[test]
    fn the_generated_crd_knows_upstream_auth_and_an_older_one_does_not() {
        let current = include_str!("../../../../deploy/crds.yaml");
        assert!(crd_has_upstream_auth(&crd_from(current)));
        let older = current.replace("upstream_auth:", "upstream_auth_removed:");
        assert!(!crd_has_upstream_auth(&crd_from(&older)));
    }
}
