//! The reconcile loop against a real ephemeral kube-apiserver.
//!
//! What the fast tier in `libs/server/controller/tests/reconcile.rs` cannot
//! cover: the status patch actually landing on the resource, the "patch only if
//! it changed" optimization being real (asserted on `resourceVersion`, which
//! only a real apiserver maintains), and the finalizer wrapper adding and
//! removing `PROXY_KUBE_FINALIZER` around a delete.
//!
//! Lives in the `api` crate because that is where the envtest harness already
//! is; `controller` is an `api` dev-dependency for this file alone.
//!
//! Gated behind the `envtest` feature so a plain `cargo test` needs no binaries.
//! Also needs the test Redis, since the reconcile writes the cache before it
//! patches — both are skipped rather than failed when unavailable, unless
//! `REQUIRE_TEST_REDIS` is set.

#![cfg(feature = "envtest")]

mod envtest_support;
mod harness;

use std::sync::Arc;
use std::time::Duration;

use common::{State, oidc_conf::OidcConf};
use controller::proxy_kube_api::{
    main_reconcile_proxy_kube_api, reconcile::reconcile_proxy_kube_api,
};
use crd::{PROXY_KUBE_FINALIZER, ProxyKubeApi};
use deadpool_redis::{Pool, redis::AsyncTypedCommands};
use envtest_support::EnvTest;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::ResourceExt;
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use kube::runtime::controller::Action;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

macro_rules! redis_or_skip {
    () => {
        match harness::try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

fn shipped_crd() -> CustomResourceDefinition {
    let yaml = include_str!("../../../../deploy/crds.yaml");
    serde_yaml::from_str(yaml).expect("the generated CRD should deserialize")
}

/// Install the shipped CRD and wait for the apiserver to serve it.
async fn install_crd(client: kube::Client) {
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crd = shipped_crd();
    let name = crd.name_any();
    crds.patch(
        &name,
        &PatchParams::apply("envtest").force(),
        &Patch::Apply(&crd),
    )
    .await
    .expect("the generated CRD should be accepted");

    for _ in 0..60 {
        if let Ok(installed) = crds.get(&name).await {
            let established = installed
                .status
                .and_then(|status| status.conditions)
                .is_some_and(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Established" && c.status == "True")
                });
            if established {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the CRD never became established");
}

/// A `State` wired to the ephemeral apiserver and the real test Redis.
fn state_for(client: kube::Client) -> State {
    State::from_parts(
        client,
        common::redis_pool::RedisPool::from_url(&harness::redis_url())
            .expect("redis pool should build"),
        OidcConf {
            client_id: "proxyauthk8s".to_string(),
            client_secret: None,
            issuer_url: "https://oidc.example.com".to_string(),
            scopes: "openid".to_string(),
            audience: "proxyauthk8s".to_string(),
            accept_authorized_party: false,
            redirect_url: None,
        },
        "https://proxy.example.com".to_string(),
        "https://front.example.com".to_string(),
    )
}

/// Wipe this cluster's Redis keys.
///
/// Each test gets a fresh apiserver but they all share one Redis, so a rerun
/// would otherwise inherit the previous run's cache and — because the retry
/// counter has a 24h TTL — its accumulated backoff.
async fn reset_redis(pool: &Pool, id: &str) {
    let mut conn = pool.get().await.expect("redis connection");
    let _ = conn.del(id).await;
    let _ = conn.del(format!("requeue_retry:{id}")).await;
    let _ = conn
        .srem(format!("{}:index", harness::REDIS_PREFIX), id)
        .await;
}

/// The Redis key a proxy in the `default` namespace is cached under.
fn cache_id(name: &str) -> String {
    format!("{}:default/{}", harness::REDIS_PREFIX, name)
}

fn proxies(client: kube::Client) -> Api<ProxyKubeApi> {
    Api::namespaced(client, "default")
}

/// Answer the reachability probe — a plain GET on the service URL.
async fn mount_probe(server: &MockServer, status_code: u16) {
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status_code))
        .mount(server)
        .await;
}

/// Create a proxy pointing at `upstream_url` on the real apiserver.
async fn create_proxy(client: kube::Client, name: &str, upstream_url: &str) -> ProxyKubeApi {
    let proxy = harness::proxy_fixture("default", name, upstream_url);
    proxies(client)
        .create(&PostParams::default(), &proxy)
        .await
        .expect("the proxy should be accepted by the apiserver")
}

async fn delete_proxy(client: kube::Client, name: &str) {
    let api = proxies(client);
    // A finalizer added by a test would block the delete forever; drop it first.
    let _ = api
        .patch(
            name,
            &PatchParams::default(),
            &Patch::Merge(serde_json::json!({ "metadata": { "finalizers": [] } })),
        )
        .await;
    let _ = api.delete(name, &DeleteParams::default()).await;
}

#[tokio::test]
async fn a_reachable_cluster_gets_its_status_patched_onto_the_resource() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let cluster = MockServer::start().await;
    mount_probe(&cluster, 200).await;

    reset_redis(&pool, &cache_id("reconcile-healthy")).await;
    let proxy = create_proxy(client.clone(), "reconcile-healthy", &cluster.uri()).await;
    let state = state_for(client.clone());

    let action = reconcile_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect("a reachable cluster reconciles");
    assert_eq!(action, Action::requeue(Duration::from_secs(60 * 60)));

    // The point of this tier: the status is really on the object, written
    // through the `status` subresource the CRD declares.
    let stored = proxies(client.clone())
        .get("reconcile-healthy")
        .await
        .expect("the proxy should still exist");
    let status = stored.status.expect("the status should have been patched");
    assert!(status.exposed);
    assert_eq!(
        status.path.as_deref(),
        Some("/clusters/default/reconcile-healthy")
    );
    assert!(status.error.is_none());

    reset_redis(&pool, &cache_id("reconcile-healthy")).await;
    delete_proxy(client, "reconcile-healthy").await;
}

#[tokio::test]
async fn an_unreachable_cluster_gets_an_error_status_patched_onto_the_resource() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let cluster = MockServer::start().await;
    mount_probe(&cluster, 500).await;

    reset_redis(&pool, &cache_id("reconcile-broken")).await;
    let proxy = create_proxy(client.clone(), "reconcile-broken", &cluster.uri()).await;
    let state = state_for(client.clone());

    let action = reconcile_proxy_kube_api(&proxy, Arc::new(state))
        .await
        .expect("an unreachable cluster still reconciles");
    // First consecutive failure: the base backoff delay.
    assert_eq!(action, Action::requeue(Duration::from_secs(5 * 60)));

    let stored = proxies(client.clone())
        .get("reconcile-broken")
        .await
        .expect("the proxy should still exist");
    let status = stored.status.expect("the status should have been patched");
    assert!(!status.exposed);
    assert!(status.path.is_none());
    assert_eq!(
        status.error.as_deref(),
        Some("Target service is not reachable")
    );

    reset_redis(&pool, &cache_id("reconcile-broken")).await;
    delete_proxy(client, "reconcile-broken").await;
}

#[tokio::test]
async fn an_unchanged_status_is_not_patched_again() {
    // The "patch only if necessary" branch exists to avoid pointless writes and
    // update conflicts. `resourceVersion` is the only honest way to check it:
    // a no-op apply still bumps it, so an unchanged version proves no write.
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let cluster = MockServer::start().await;
    mount_probe(&cluster, 200).await;

    reset_redis(&pool, &cache_id("reconcile-stable")).await;
    let proxy = create_proxy(client.clone(), "reconcile-stable", &cluster.uri()).await;
    let state = state_for(client.clone());

    reconcile_proxy_kube_api(&proxy, Arc::new(state.clone()))
        .await
        .expect("the first reconcile writes the status");
    let after_first = proxies(client.clone())
        .get("reconcile-stable")
        .await
        .expect("the proxy should still exist");
    let version_after_first = after_first.resource_version();

    // Feed the freshly patched object back in, exactly as the watch would.
    reconcile_proxy_kube_api(&after_first, Arc::new(state))
        .await
        .expect("the second reconcile succeeds");

    let after_second = proxies(client.clone())
        .get("reconcile-stable")
        .await
        .expect("the proxy should still exist");
    assert_eq!(
        after_second.resource_version(),
        version_after_first,
        "an unchanged status must not be patched again"
    );

    reset_redis(&pool, &cache_id("reconcile-stable")).await;
    delete_proxy(client, "reconcile-stable").await;
}

#[tokio::test]
async fn the_controller_entrypoint_adds_the_finalizer_and_removes_it_on_delete() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let cluster = MockServer::start().await;
    mount_probe(&cluster, 200).await;

    reset_redis(&pool, &cache_id("reconcile-finalizer")).await;
    let proxy = create_proxy(client.clone(), "reconcile-finalizer", &cluster.uri()).await;
    let state = state_for(client.clone());
    state
        .is_leader
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let ctx = Arc::new(state);

    // Pass 1: the object carries no finalizer yet, so `kube`'s finalizer wrapper
    // only adds one — the apply deliberately waits for the watch event that
    // patch produces, so the cache is not written until the object is
    // guaranteed to be cleaned up on delete.
    main_reconcile_proxy_kube_api(Arc::new(proxy), ctx.clone())
        .await
        .expect("the entrypoint adds the finalizer");

    let stored = proxies(client.clone())
        .get("reconcile-finalizer")
        .await
        .expect("the proxy should still exist");
    assert!(
        stored
            .finalizers()
            .contains(&PROXY_KUBE_FINALIZER.to_string()),
        "the finalizer should have been added, got {:?}",
        stored.finalizers()
    );
    let id = stored.to_identifier();
    assert!(
        harness::cached_value(&pool, &id).await.is_none(),
        "nothing should be cached before the finalizer is in place"
    );

    // Pass 2: the finalizer is present, so this one actually applies.
    main_reconcile_proxy_kube_api(Arc::new(stored), ctx.clone())
        .await
        .expect("the entrypoint reconciles");
    assert!(
        harness::cached_value(&pool, &id).await.is_some(),
        "the reconcile should have cached the cluster"
    );

    // Deleting only marks the object; the finalizer keeps it alive until the
    // controller has cleaned the cache up.
    proxies(client.clone())
        .delete("reconcile-finalizer", &DeleteParams::default())
        .await
        .expect("the delete should be accepted");
    let marked = proxies(client.clone())
        .get("reconcile-finalizer")
        .await
        .expect("the finalizer should keep the object alive");
    assert!(marked.metadata.deletion_timestamp.is_some());

    // Pass 3: the object is deleting, so the wrapper runs the cleanup and only
    // then releases the finalizer.
    main_reconcile_proxy_kube_api(Arc::new(marked), ctx)
        .await
        .expect("the entrypoint cleans up");

    assert!(
        proxies(client.clone())
            .get("reconcile-finalizer")
            .await
            .is_err(),
        "the object should be gone once the finalizer is released"
    );
    assert!(
        harness::cached_value(&pool, &id).await.is_none(),
        "the cleanup should have dropped the cached cluster"
    );
}

#[tokio::test]
async fn a_follower_does_not_touch_the_resource() {
    // The lease is what prevents two replicas from patching the same object;
    // a follower must add no finalizer and write no status.
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_crd(client.clone()).await;

    let cluster = MockServer::start().await;
    mount_probe(&cluster, 200).await;

    reset_redis(&pool, &cache_id("reconcile-follower")).await;
    let proxy = create_proxy(client.clone(), "reconcile-follower", &cluster.uri()).await;
    let state = state_for(client.clone());
    state
        .is_leader
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let action = main_reconcile_proxy_kube_api(Arc::new(proxy), Arc::new(state))
        .await
        .expect("a follower short-circuits successfully");
    assert_eq!(action, Action::requeue(Duration::from_secs(20)));

    let stored = proxies(client.clone())
        .get("reconcile-follower")
        .await
        .expect("the proxy should still exist");
    assert!(
        stored.status.is_none(),
        "a follower must not write a status"
    );
    assert!(
        stored.finalizers().is_empty(),
        "a follower must not add a finalizer"
    );

    reset_redis(&pool, &cache_id("reconcile-follower")).await;
    delete_proxy(client, "reconcile-follower").await;
}
