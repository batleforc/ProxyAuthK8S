//! The `OneShotToken` bootstrap Secret against a real apiserver (RFC 0012
//! §6.4): written once with its Redis mark, kept while unused, replaced when
//! spent under `auto_rotate` or on the `regenerate-bootstrap` annotation.
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

use common::agent_gateway::bootstrap_key;
use controller::proxy_kube_api::bootstrap::{TOKEN_KEY, ensure_one_shot_token};
use crd::service::{OneShotTokenBootstrap, REGENERATE_BOOTSTRAP_ANNOTATION};
use envtest_support::EnvTest;
use k8s_openapi::api::core::v1::Secret;
use kube::Api;

macro_rules! envtest_or_skip {
    () => {
        match EnvTest::try_start().await {
            Some(env_test) => env_test,
            None => return,
        }
    };
}

async fn token(secrets: &Api<Secret>, name: &str) -> String {
    let secret = secrets.get(name).await.expect("the bootstrap Secret");
    let data = secret.data.expect("data");
    String::from_utf8(data.get(TOKEN_KEY).expect("token").0.clone()).expect("utf-8")
}

#[tokio::test]
async fn the_bootstrap_token_is_written_kept_and_rotated() {
    let env_test = envtest_or_skip!();
    if harness::try_redis_pool().await.is_none() {
        return;
    }
    let mut state = harness::test_state("https://idp.example.com".to_string());
    state.client = env_test.client().expect("client should build");
    let (ns, name) = harness::unique_cluster();
    let mut proxy = harness::proxy_fixture(&ns, &name, "https://unused.example.com");
    let uid = format!("{:08x}-0000-4000-8000-00000000b007", std::process::id());
    proxy.metadata.uid = Some(uid.clone());
    let secrets: Api<Secret> = Api::namespaced(state.client.clone(), &ns);
    let secret = crd::service::bootstrap_secret_name(&name);
    let once = OneShotTokenBootstrap { auto_rotate: false };

    assert!(
        ensure_one_shot_token(&proxy, &once, &state)
            .await
            .expect("written")
    );
    let first = token(&secrets, &secret).await;
    assert_eq!(first.len(), 43, "32 random bytes, base64url");
    assert!(
        state
            .key_exists(&bootstrap_key(&uid, &first))
            .await
            .expect("redis")
    );
    let owner = secrets
        .get(&secret)
        .await
        .expect("secret")
        .metadata
        .owner_references;
    assert_eq!(owner.expect("owner")[0].uid, uid);

    // Unused: kept. Spent without auto_rotate: kept, reported unusable.
    assert!(
        ensure_one_shot_token(&proxy, &once, &state)
            .await
            .expect("kept")
    );
    assert_eq!(token(&secrets, &secret).await, first);
    state
        .redis_take(&bootstrap_key(&uid, &first))
        .await
        .expect("redis");
    assert!(
        !ensure_one_shot_token(&proxy, &once, &state)
            .await
            .expect("spent")
    );
    assert_eq!(token(&secrets, &secret).await, first);

    // Spent under auto_rotate: a new one.
    let rotating = OneShotTokenBootstrap { auto_rotate: true };
    assert!(
        ensure_one_shot_token(&proxy, &rotating, &state)
            .await
            .expect("rotated")
    );
    let second = token(&secrets, &secret).await;
    assert_ne!(second, first);

    // Asked by annotation: a new one, once per value; the old mark is gone.
    proxy
        .metadata
        .annotations
        .get_or_insert_default()
        .insert(REGENERATE_BOOTSTRAP_ANNOTATION.to_string(), "1".to_string());
    assert!(
        ensure_one_shot_token(&proxy, &once, &state)
            .await
            .expect("asked")
    );
    let third = token(&secrets, &secret).await;
    assert_ne!(third, second);
    assert!(
        !state
            .key_exists(&bootstrap_key(&uid, &second))
            .await
            .expect("redis")
    );
    assert!(
        ensure_one_shot_token(&proxy, &once, &state)
            .await
            .expect("served")
    );
    assert_eq!(token(&secrets, &secret).await, third);
}
