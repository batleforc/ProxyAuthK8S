//! Leader election against a real apiserver (coordination.k8s.io is served by
//! the bare apiserver envtest starts).
//!
//! What matters is split-brain: at most one replica may believe it is the
//! leader, a dead leader must be replaced once its Lease expires, and a leader
//! whose Lease was taken over must step down. The Lease TTL and renew period
//! are shortened so a handover takes seconds; every assertion polls with a
//! timeout. Needs no Redis. Gated behind the `envtest` feature.
#![cfg_attr(
    feature = "envtest",
    expect(
        clippy::expect_used,
        reason = "test helpers: a panic is a failed test, with its message"
    )
)]
#![cfg(feature = "envtest")]

mod envtest_support;
mod harness;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use common::{State, oidc_conf::OidcConf, redis_pool::RedisPool};
use controller::{LEASE_NAME, error::ControllerError, lease_lock, run, run_leader_election};
use envtest_support::{EnvTest, wait_until};
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use kube::api::{Api, Patch, PatchParams};
use serde_json::json;

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

const NAMESPACE: &str = "default";
const TTL: Duration = Duration::from_secs(2);
const RENEW_EVERY: Duration = Duration::from_millis(200);
/// A TTL plus a few renew periods, with room for a slow machine.
const HANDOVER: Duration = Duration::from_secs(15);

/// A `State` on the envtest apiserver. Redis is never contacted here.
fn replica_state(client: kube::Client) -> State {
    State::from_parts(
        client,
        RedisPool::from_url(&harness::redis_url()).expect("redis pool should build"),
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

/// A replica running the election loop; stops (like a killed pod) on drop.
struct Replica {
    state: State,
    election: tokio::task::JoinHandle<()>,
}

impl Replica {
    fn start(client: &kube::Client, holder: &str) -> Self {
        let state = replica_state(client.clone());
        let lock = Arc::new(lease_lock(client.clone(), NAMESPACE, holder, TTL));
        let election = tokio::spawn(run_leader_election(state.clone(), lock, RENEW_EVERY, TTL));
        Self { state, election }
    }

    fn is_leader(&self) -> bool {
        self.state.is_leader.load(Ordering::Relaxed)
    }
}

impl Drop for Replica {
    fn drop(&mut self) {
        self.election.abort();
    }
}

async fn holder(client: &kube::Client) -> Option<String> {
    Api::<Lease>::namespaced(client.clone(), NAMESPACE)
        .get_opt(LEASE_NAME)
        .await
        .expect("the apiserver should answer")
        .and_then(|lease| lease.spec)
        .and_then(|spec| spec.holder_identity)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_leader_is_elected_and_replaced_when_it_dies() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");

    let first = Replica::start(&client, "replica-a");
    wait_until(HANDOVER, "replica-a to take the Lease", || async {
        first.is_leader()
    })
    .await;

    let second = Replica::start(&client, "replica-b");
    // Several renew periods: the follower must keep failing to acquire.
    for _ in 0..10 {
        tokio::time::sleep(RENEW_EVERY).await;
        assert!(first.is_leader(), "the live leader must keep its Lease");
        assert!(!second.is_leader(), "two replicas must never both lead");
    }
    assert_eq!(holder(&client).await.as_deref(), Some("replica-a"));

    // The leader dies without releasing the Lease: the follower takes over
    // once it expires.
    drop(first);
    wait_until(
        HANDOVER,
        "replica-b to take over the expired Lease",
        || async { second.is_leader() },
    )
    .await;
    assert_eq!(holder(&client).await.as_deref(), Some("replica-b"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_leader_whose_lease_is_taken_over_steps_down() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");

    let replica = Replica::start(&client, "replica-a");
    wait_until(HANDOVER, "replica-a to take the Lease", || async {
        replica.is_leader()
    })
    .await;

    // Another replica now holds a fresh, long Lease (e.g. after a partition
    // during which this one could not renew in time).
    let now = MicroTime(k8s_openapi::jiff::Timestamp::now());
    Api::<Lease>::namespaced(client.clone(), NAMESPACE)
        .patch(
            LEASE_NAME,
            &PatchParams::default(),
            &Patch::Merge(json!({
                "spec": {
                    "holderIdentity": "replica-z",
                    "leaseDurationSeconds": 3600,
                    "acquireTime": now,
                    "renewTime": now,
                }
            })),
        )
        .await
        .expect("the Lease should be patched");

    wait_until(HANDOVER, "replica-a to step down", || async {
        !replica.is_leader()
    })
    .await;
    // And it must not take the Lease back while it is valid.
    for _ in 0..5 {
        tokio::time::sleep(RENEW_EVERY).await;
        assert!(!replica.is_leader());
    }
    assert_eq!(holder(&client).await.as_deref(), Some("replica-z"));
}

/// A TCP relay to the apiserver that can be frozen: once frozen, open
/// connections are dropped and new ones are accepted but never answered, so
/// every request through it fails or hangs, as for a leader stuck in a pause.
struct Relay {
    url: String,
    frozen: Arc<AtomicBool>,
    accept: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start(upstream_url: &str) -> Self {
        let upstream = upstream_url
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("relay should bind");
        let url = format!("https://{}", listener.local_addr().expect("local addr"));
        let frozen = Arc::new(AtomicBool::new(false));
        let accept = tokio::spawn({
            let frozen = frozen.clone();
            async move {
                let mut open = Vec::new();
                loop {
                    let Ok((mut inbound, _)) = listener.accept().await else {
                        return;
                    };
                    if frozen.load(Ordering::Relaxed) {
                        // Hold it open, silent.
                        open.push(tokio::spawn(async move {
                            std::future::pending::<()>().await;
                            drop(inbound);
                        }));
                        continue;
                    }
                    let upstream = upstream.clone();
                    let frozen = frozen.clone();
                    open.push(tokio::spawn(async move {
                        let Ok(mut outbound) = tokio::net::TcpStream::connect(&upstream).await
                        else {
                            return;
                        };
                        tokio::select! {
                            _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound) => {}
                            () = async {
                                while !frozen.load(Ordering::Relaxed) {
                                    tokio::time::sleep(Duration::from_millis(20)).await;
                                }
                            } => {}
                        }
                    }));
                }
            }
        });
        Self {
            url,
            frozen,
            accept,
        }
    }

    fn client(&self, token: &str) -> kube::Client {
        let mut config = kube::Config::new(self.url.parse().expect("relay url"));
        config.accept_invalid_certs = true;
        config.auth_info.token = Some(secrecy::SecretBox::new(token.to_string().into()));
        kube::Client::try_from(config).expect("relay client")
    }

    fn freeze(&self) {
        self.frozen.store(true, Ordering::Relaxed);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

/// RFC 0011 §6.P point 14 (T7): a leader whose renewals stall past the TTL
/// steps down, instead of staying leader next to the replica that took over
/// (what `kube-leader-election`, with no deadline and no `resourceVersion`,
/// allowed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_frozen_leader_steps_down_before_its_lease_can_be_taken() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    let relay = Relay::start(env_test.url()).await;

    let frozen = Replica::start(&relay.client(env_test.token()), "replica-frozen");
    wait_until(HANDOVER, "replica-frozen to take the Lease", || async {
        frozen.is_leader()
    })
    .await;

    relay.freeze();
    // `hold` gives up `TTL - RENEW_EVERY / 2` after its last renewal: before
    // another replica may take the Lease.
    wait_until(TTL * 2, "the frozen leader to step down", || async {
        !frozen.is_leader()
    })
    .await;

    let other = Replica::start(&client, "replica-b");
    wait_until(HANDOVER, "replica-b to take over", || async {
        other.is_leader()
    })
    .await;
    assert!(!frozen.is_leader(), "two replicas must never both lead");
    assert_eq!(holder(&client).await.as_deref(), Some("replica-b"));
}

#[tokio::test]
async fn run_refuses_to_start_without_the_crd() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");
    // A fresh apiserver: the ProxyKubeApi CRD is not installed.
    let result = tokio::time::timeout(Duration::from_secs(30), run(replica_state(client)))
        .await
        .expect("run must fail fast without the CRD");
    assert!(
        matches!(result, Err(ControllerError::CrdUnavailable(_))),
        "{result:?}"
    );
}
