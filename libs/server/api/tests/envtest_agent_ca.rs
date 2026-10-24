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
