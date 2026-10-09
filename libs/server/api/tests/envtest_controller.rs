//! The `ProxyKubeApi` controller, run for real against envtest + Redis.
//!
//! `controller::run` is started as-is: it lists the CRD, takes the leader
//! Lease (coordination.k8s.io is served by the bare apiserver) and runs the
//! reconcile loop with its finalizer. The tests then drive a resource through
//! its life — create, update, delete — and check the two places the
//! controller writes to: Redis (what the request path reads) and the CR
//! status (what the operator sees).
//!
//! Every assertion polls with a timeout, so a slow machine only makes the run
//! slower, never red. Gated behind the `envtest` feature.

#![cfg(feature = "envtest")]

mod envtest_support;
mod harness;

use std::time::Duration;

use common::{State, oidc_conf::OidcConf, redis_pool::RedisPool};
use crd::{PROXY_KUBE_FINALIZER, ProxyKubeApi};
use deadpool_redis::redis::AsyncTypedCommands;
use envtest_support::{EnvTest, install_shipped_crd, wait_until};
use harness::{REDIS_PREFIX, redis_url, try_redis_pool, unique_cluster};
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, DeleteParams, ObjectMeta, Patch, PatchParams, PostParams};
use serde_json::json;
use wiremock::MockServer;

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
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

const NAMESPACE: &str = "default";
/// Fixed by `controller::run`.
const LEASE_NAME: &str = "proxy-auth-k8s-leader-election";
/// Generous: a reconcile is a reachability probe plus two writes.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A `State` wired to the envtest apiserver and the test Redis.
fn controller_state(env_test: &EnvTest, holder: &str) -> State {
    let mut state = State::from_parts(
        env_test.client().expect("client should build"),
        RedisPool::from_url(&redis_url()).expect("redis pool should build"),
        OidcConf {
            client_id: "proxyauthk8s".to_string(),
            client_secret: None,
            issuer_url: "http://127.0.0.1:1".to_string(),
            scopes: "openid".to_string(),
            audience: "proxyauthk8s".to_string(),
            accept_authorized_party: false,
            redirect_url: None,
        },
        "https://proxy.example.com".to_string(),
        "https://front.example.com".to_string(),
    );
    state.lease_namespace = NAMESPACE.to_string();
    state.lease_name = holder.to_string();
    state
}

/// Run the controller in the background; aborted when the guard drops.
struct RunningController(tokio::task::JoinHandle<()>);

impl RunningController {
    fn start(state: State) -> Self {
        Self(tokio::spawn(async move {
            controller::run(state)
                .await
                .expect("the controller should start once the CRD is installed");
        }))
    }
}

impl Drop for RunningController {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn proxy_resource(name: &str, upstream: &str) -> ProxyKubeApi {
    serde_json::from_value(json!({
        "apiVersion": "weebo.si.rs/v1",
        "kind": "ProxyKubeApi",
        "metadata": { "name": name, "namespace": NAMESPACE },
        "spec": {
            "enabled": true,
            "cert": { "Insecure": true },
            "service": { "ExternalService": { "url": upstream } },
        },
    }))
    .expect("resource should deserialize")
}

async fn cached(state: &State, name: &str) -> Option<ProxyKubeApi> {
    state
        .get_object_from_redis(REDIS_PREFIX, &format!("{NAMESPACE}/{name}"))
        .await
        .expect("redis should answer")
}

async fn indexed(pool: &deadpool_redis::Pool, name: &str) -> bool {
    let mut conn = pool.get().await.expect("redis connection");
    conn.sismember(
        format!("{REDIS_PREFIX}:index"),
        format!("{REDIS_PREFIX}:{NAMESPACE}/{name}"),
    )
    .await
    .expect("redis should answer")
}

fn retry_key(name: &str) -> String {
    format!("requeue_retry:{REDIS_PREFIX}:{NAMESPACE}/{name}")
}

/// Create → Redis + status + finalizer; disable → Redis follows; unreachable
/// upstream → status reports it; delete → finalizer cleans Redis and the
/// object goes away.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_controller_mirrors_a_proxy_through_its_lifecycle() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_shipped_crd(client.clone()).await;

    // Any 2xx/4xx answer counts as reachable; wiremock 404s unmatched paths.
    let upstream = MockServer::start().await;
    let (_, name) = unique_cluster();
    let state = controller_state(&env_test, &format!("holder-{name}"));
    let _controller = RunningController::start(state.clone());

    let proxies: Api<ProxyKubeApi> = Api::namespaced(client.clone(), NAMESPACE);
    proxies
        .create(
            &PostParams::default(),
            &proxy_resource(&name, &upstream.uri()),
        )
        .await
        .expect("the resource should be accepted");

    // 1. Applied: cached, indexed, exposed, finalizer set.
    let expected_path = format!("/clusters/{NAMESPACE}/{name}");
    wait_until(TIMEOUT, "the proxy to be cached as exposed", || async {
        cached(&state, &name)
            .await
            .and_then(|proxy| proxy.status)
            .is_some_and(|status| status.exposed)
    })
    .await;
    let in_redis = cached(&state, &name).await.expect("cached");
    assert!(in_redis.spec.enabled);
    let status = in_redis.status.expect("status is cached too");
    assert_eq!(status.path.as_deref(), Some(expected_path.as_str()));
    assert_eq!(status.error, None);
    assert!(indexed(&pool, &name).await, "the proxy should be indexed");

    wait_until(TIMEOUT, "the CR status to report exposed", || async {
        proxies
            .get(&name)
            .await
            .ok()
            .and_then(|proxy| proxy.status)
            .is_some_and(|status| {
                status.exposed && status.path.as_deref() == Some(expected_path.as_str())
            })
    })
    .await;
    let live = proxies.get(&name).await.expect("the CR exists");
    assert!(
        live.metadata
            .finalizers
            .unwrap_or_default()
            .iter()
            .any(|f| f == PROXY_KUBE_FINALIZER),
        "the controller should add its finalizer"
    );

    // 2. Disabled: Redis follows the spec (the request path then answers 404).
    proxies
        .patch(
            &name,
            &PatchParams::default(),
            &Patch::Merge(json!({ "spec": { "enabled": false } })),
        )
        .await
        .expect("the update should be accepted");
    wait_until(TIMEOUT, "Redis to see the proxy disabled", || async {
        cached(&state, &name)
            .await
            .is_some_and(|proxy| !proxy.spec.enabled)
    })
    .await;

    // 3. Unreachable upstream: the status says so, in the CR and in Redis.
    proxies
        .patch(
            &name,
            &PatchParams::default(),
            &Patch::Merge(json!({
                "spec": { "service": { "ExternalService": { "url": "http://127.0.0.1:1" } } }
            })),
        )
        .await
        .expect("the update should be accepted");
    wait_until(TIMEOUT, "the CR status to report the failure", || async {
        proxies
            .get(&name)
            .await
            .ok()
            .and_then(|proxy| proxy.status)
            .is_some_and(|status| !status.exposed && status.error.is_some())
    })
    .await;
    let in_redis = cached(&state, &name).await.expect("still cached");
    let status = in_redis.status.expect("status is cached");
    assert!(!status.exposed);
    assert!(
        status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("reachable")),
        "unexpected error: {:?}",
        status.error
    );
    assert!(
        state
            .key_exists(&retry_key(&name))
            .await
            .expect("redis should answer"),
        "a failed reconcile should count towards the backoff"
    );

    // 4. Deleted: the finalizer cleans Redis, then the object is reaped.
    proxies
        .delete(&name, &DeleteParams::default())
        .await
        .expect("the delete should be accepted");
    wait_until(TIMEOUT, "the object to be reaped", || async {
        matches!(
            proxies.get(&name).await,
            Err(kube::Error::Api(status)) if status.code == 404
        )
    })
    .await;
    assert!(
        cached(&state, &name).await.is_none(),
        "cleanup should drop the cache"
    );
    assert!(
        !indexed(&pool, &name).await,
        "cleanup should drop the index entry"
    );
    assert!(
        !state
            .key_exists(&retry_key(&name))
            .await
            .expect("redis should answer"),
        "cleanup should drop the retry counter, or a recreated proxy inherits its backoff"
    );
}

/// A replica that does not hold the Lease must leave everything alone: no
/// finalizer, no Redis write. Otherwise two replicas would reconcile at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_follower_does_not_reconcile() {
    let pool = redis_or_skip!();
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    install_shipped_crd(client.clone()).await;

    // Someone else holds a fresh, long Lease.
    let leases: Api<Lease> = Api::namespaced(client.clone(), NAMESPACE);
    leases
        .create(
            &PostParams::default(),
            &Lease {
                metadata: ObjectMeta {
                    name: Some(LEASE_NAME.to_string()),
                    ..ObjectMeta::default()
                },
                spec: Some(k8s_openapi::api::coordination::v1::LeaseSpec {
                    holder_identity: Some("another-replica".to_string()),
                    lease_duration_seconds: Some(3600),
                    acquire_time: Some(MicroTime(k8s_openapi::jiff::Timestamp::now())),
                    renew_time: Some(MicroTime(k8s_openapi::jiff::Timestamp::now())),
                    ..Default::default()
                }),
            },
        )
        .await
        .expect("the Lease should be created");

    let upstream = MockServer::start().await;
    let (_, name) = unique_cluster();
    let state = controller_state(&env_test, &format!("follower-{name}"));
    let _controller = RunningController::start(state.clone());

    let proxies: Api<ProxyKubeApi> = Api::namespaced(client.clone(), NAMESPACE);
    proxies
        .create(
            &PostParams::default(),
            &proxy_resource(&name, &upstream.uri()),
        )
        .await
        .expect("the resource should be accepted");

    // A leader does all of this well under a second here; give a follower
    // three times that to (wrongly) act.
    tokio::time::sleep(Duration::from_secs(3)).await;

    assert!(
        !state.is_leader.load(std::sync::atomic::Ordering::Relaxed),
        "the controller must not claim a Lease held by another replica"
    );
    let live = proxies.get(&name).await.expect("the CR exists");
    assert!(
        live.metadata.finalizers.unwrap_or_default().is_empty(),
        "a follower must not add the finalizer"
    );
    assert!(
        live.status.is_none(),
        "a follower must not write the status"
    );
    assert!(
        cached(&state, &name).await.is_none(),
        "a follower must not write Redis"
    );
    assert!(!indexed(&pool, &name).await);
}
