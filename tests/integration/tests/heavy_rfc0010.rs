//! RFC 0010 with a cluster IdP other than Dex: Authentik exchanges the proxy
//! token (Dex A's) for its own, Dex A federated into it by an OIDC source
//! (`.it/heavy/authentik.yaml`). Authentik verifies the token by Dex A's JWKS
//! and names a service account it generates, `kube-ak-<sub of the token>`;
//! the apiserver trusts Authentik's issuer and reads that name, prefixed
//! `authentik:` (`.it/k3d.yaml.tpl`). `task it:heavy` runs them.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use integration::{
    Bench, Persona,
    fixture::{self, CLUSTER, ExchangeIdp},
    heavy::{self, AUTHENTIK_CLIENT, AUTHENTIK_SECRET},
    observe,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// The `sub` of a JWT, read without verifying it: the test's own token.
fn subject(jwt: &str) -> String {
    let payload = jwt.split('.').nth(1).expect("a JWT");
    let claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).expect("base64url payload"))
            .expect("JSON claims");
    claims["sub"].as_str().expect("a sub").to_string()
}

/// Authentik's exchange for the cluster, with `secret` as its client
/// secret; `username` is made admin of the namespace in the apiserver.
fn authentik(secret: &str, username: &str) -> ExchangeIdp {
    ExchangeIdp {
        issuer_url: heavy::authentik_issuer(),
        client_id: AUTHENTIK_CLIENT.to_string(),
        client_secret: secret.to_string(),
        connector_id: String::new(),
        // Without a scope, Authentik's token carries no claim but `sub`.
        scope: "openid email profile".to_string(),
        subject: json!({ "apiGroup": "rbac.authorization.k8s.io",
                         "kind": "User", "name": username }),
    }
}

/// The results of the `token_exchanged` lines of `ns`.
fn exchanges(ns: &str) -> Vec<String> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|line| line["ns"] == ns && line["event"] == "token_exchanged")
        .map(|line| line["result"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Point 1: the reader's proxy token reaches a cluster whose IdP is
/// Authentik — the proxy exchanged it once, and the apiserver sees the
/// account Authentik generated for it, not the proxy's user.
#[tokio::test]
async fn p1_authentik_exchanges_the_proxy_token() {
    heavy::deploy_authentik().await;
    let token = Persona::named("reader").dashboard_token().await;
    let username = format!("authentik:{AUTHENTIK_CLIENT}-{}", subject(&token));
    let guard = fixture::token_exchange_with(
        "heavy-0010-p1",
        &authentik(AUTHENTIK_SECRET, &username),
        Value::Null,
        300,
    );
    let ns = guard.name.as_str();
    let bench = Bench::get();

    // The apiserver's JWT authenticator loads Authentik's discovery on its own
    // retry loop, so it answers 401 for a while after Authentik first serves
    // it. The retries hit the proxy's cache: still one exchange.
    let start = Instant::now();
    let pods = loop {
        let pods = bench
            .proxied(
                Method::GET,
                ns,
                CLUSTER,
                &format!("/api/v1/namespaces/{ns}/pods"),
                &token,
            )
            .send()
            .await
            .expect("list pods");
        if pods.status() != StatusCode::UNAUTHORIZED || start.elapsed() > Duration::from_secs(60) {
            break pods;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    assert_eq!(
        pods.status(),
        StatusCode::OK,
        "{exchanges:?}",
        exchanges = exchanges(ns)
    );

    let review: Value = bench
        .proxied(
            Method::POST,
            ns,
            CLUSTER,
            "/apis/authentication.k8s.io/v1/selfsubjectreviews",
            &token,
        )
        .json(&json!({ "apiVersion": "authentication.k8s.io/v1", "kind": "SelfSubjectReview" }))
        .send()
        .await
        .expect("SelfSubjectReview")
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        review["status"]["userInfo"]["username"], username,
        "{review}"
    );
    assert_eq!(exchanges(ns), ["exchanged"], "one exchange, then the cache");
}

/// Point 2 (refusal): the cluster's client secret is wrong — Authentik
/// refuses the client, the proxy answers `401` without relaying the proxy
/// token, and writes a `refused` exchange.
#[tokio::test]
async fn p2_a_client_authentik_does_not_know_is_refused() {
    heavy::deploy_authentik().await;
    let token = Persona::named("reader").dashboard_token().await;
    let username = format!("authentik:{AUTHENTIK_CLIENT}-{}", subject(&token));
    let guard = fixture::token_exchange_with(
        "heavy-0010-p2",
        &authentik("not-the-secret", &username),
        Value::Null,
        300,
    );
    let ns = guard.name.as_str();
    let response = Bench::get()
        .proxied(
            Method::GET,
            ns,
            CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            &token,
        )
        .send()
        .await
        .expect("list pods");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(exchanges(ns), ["refused"]);
}
