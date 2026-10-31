//! RFC 0016 §6.P points 1 and 2: what the proxy relays upstream, read from
//! the upstream echo's transcript rather than inferred from the apiserver's
//! answer. `task it:heavy` runs them.

#![expect(
    clippy::expect_used,
    reason = "test harness: a panic is a failed test, with its message"
)]

use integration::{
    Bench,
    fixture::{self, CLUSTER},
    heavy,
};

/// The identity headers a client tries to smuggle upstream.
const SMUGGLED: [(&str, &str); 5] = [
    ("Impersonate-User", "system:admin"),
    ("Impersonate-Group", "system:masters"),
    ("X-Remote-User", "system:admin"),
    ("X-Remote-Group", "system:masters"),
    ("X-Forwarded-User", "admin"),
];

/// A GET of `path` on the cluster of `ns` through the Ingress, with the
/// reader's token and every [`SMUGGLED`] header: its status.
async fn smuggle(ns: &str, path: &str, token: &str) -> u16 {
    let mut request = Bench::get().proxied(reqwest::Method::GET, ns, CLUSTER, path, token);
    for (name, value) in SMUGGLED {
        request = request.header(name, value);
    }
    request
        .header("X-HTTP-Method-Override", "DELETE")
        .send()
        .await
        .expect("request through the proxy")
        .status()
        .as_u16()
}

/// Point 1: in `Passthrough`, the user's token is relayed and nothing the
/// client added to name someone else is: no `Impersonate-*`, no
/// `X-Remote-*`, no method override, and `X-Forwarded-User` is the proxy's.
#[tokio::test]
async fn p1_passthrough_relays_the_token_and_no_smuggled_identity() {
    let p = heavy::echoed("heavy-0016-p1").await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    let path = format!("/api/v1/namespaces/{ns}/configmaps?marker=p1");
    assert_eq!(smuggle(&ns, &path, &token).await, 200);

    let seen = heavy::relayed(&ns, "marker=p1");
    assert_eq!(seen.len(), 1, "one request relayed: {seen:?}");
    let relayed = &seen[0];
    assert_eq!(relayed.method, "GET");
    assert_eq!(
        relayed.header("authorization"),
        vec![format!("Bearer {token}").as_str()],
        "Passthrough relays the user's token"
    );
    for (name, _) in SMUGGLED.iter().take(4) {
        assert!(
            relayed.header(name).is_empty(),
            "{name} reached upstream: {relayed:?}"
        );
    }
    assert!(relayed.header("x-http-method-override").is_empty());
    assert_ne!(relayed.header("x-forwarded-user"), vec!["admin"]);
}

/// Point 2: in `FrontProxy`, the same request reaches the echo with the
/// proxy's assertion alone: one `X-Remote-User`, the reader's, prefixed;
/// prefixed groups only; no token and no `Impersonate-*`.
#[tokio::test]
async fn p2_front_proxy_relays_only_the_asserted_identity() {
    let p = heavy::echoed("heavy-0016-p2").await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, CLUSTER).await;
    fixture::use_front_proxy(&p, &format!("external:{}", heavy::echo_url(&ns)));
    fixture::wait_trust(&ns, "Trusted").await;
    let path = format!("/api/v1/namespaces/{ns}/configmaps?marker=p2");
    assert_eq!(smuggle(&ns, &path, &token).await, 200);

    let seen = heavy::relayed(&ns, "marker=p2");
    assert_eq!(seen.len(), 1, "one request relayed: {seen:?}");
    let relayed = &seen[0];
    assert_eq!(
        relayed.header("x-remote-user"),
        vec!["proxyauth:reader@it.local"]
    );
    let groups = relayed.header("x-remote-group");
    assert!(
        !groups.is_empty() && groups.iter().all(|g| g.starts_with("proxyauth:")),
        "groups not all prefixed: {groups:?}"
    );
    assert!(relayed.header("authorization").is_empty(), "{relayed:?}");
    assert!(relayed.header("impersonate-user").is_empty());
    assert!(relayed.header("impersonate-group").is_empty());
    assert!(relayed.header("x-http-method-override").is_empty());
}
