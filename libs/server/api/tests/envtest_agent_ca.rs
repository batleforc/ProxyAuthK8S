//! The agent CA Secret against a real apiserver (RFC 0012, decision 26):
//! gateway pods racing on a missing Secret end up with one CA, and an
//! existing one is never replaced.
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

use api::agent::ca::load_or_create;
use envtest_support::EnvTest;

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

#[tokio::test]
async fn racing_gateway_pods_share_one_ca_and_never_replace_it() {
    let env_test = envtest_or_skip!();
    let client = env_test.client().expect("client should build");

    let racers: Vec<_> = (0..4)
        .map(|_| tokio::spawn(load_or_create(client.clone(), "default")))
        .collect();
    let mut certs = Vec::new();
    for racer in racers {
        let ca = racer
            .await
            .expect("the task should not panic")
            .expect("every racer should end up with a CA");
        certs.push(ca.cert_pem().to_string());
    }
    certs.dedup();
    assert_eq!(certs.len(), 1, "one CA for every pod");

    let again = load_or_create(client, "default")
        .await
        .expect("an existing CA should load");
    assert_eq!(again.cert_pem(), certs.first().expect("one cert").as_str());
}

/// A free loopback port, released for the gateway to bind.
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("a free port")
}

/// Step 5 of RFC 0012 §17: the `gateway` role loads the CA from a real
/// apiserver, serves `/readyz` and a token-guarded `CONNECT`, and stops on
/// its shutdown signal.
#[tokio::test]
async fn the_gateway_role_serves_until_its_shutdown() {
    let env_test = envtest_or_skip!();
    if harness::try_redis_pool().await.is_none() {
        return;
    }
    let mut state = harness::test_state("https://idp.example.com".to_string());
    state.client = env_test.client().expect("client should build");
    let mut settings = common::config::Config::from_lookup(|_| None).agent_gateway;
    settings.token = Some("t0ken".to_string());
    settings.namespace = "default".to_string();
    settings.port = free_port();
    settings.connect_port = free_port();
    let (port, connect_port) = (settings.port, settings.connect_port);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let gateway = tokio::spawn(api::agent::run(
        state,
        settings,
        "gw-run".to_string(),
        async {
            drop(stopped.await);
        },
    ));

    let ready = format!("http://127.0.0.1:{port}/readyz");
    envtest_support::wait_until(
        std::time::Duration::from_secs(10),
        "the gateway's /readyz",
        || {
            let ready = ready.clone();
            async move {
                reqwest::get(&ready)
                    .await
                    .is_ok_and(|response| response.status().is_success())
            }
        },
    )
    .await;
    let refused = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{connect_port}")).expect("proxy"))
        .build()
        .expect("client")
        .get("https://kubernetes.default.svc/")
        .send()
        .await;
    assert!(refused.is_err(), "a CONNECT without the token gets through");

    let _sent = stop.send(());
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), gateway)
        .await
        .expect("the gateway should stop on its signal")
        .expect("the gateway task should not panic");
    assert!(ended.is_ok(), "{ended:?}");
}
