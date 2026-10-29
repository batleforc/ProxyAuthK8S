//! RFC 0014 §6.P, phase 1: the `upstream_auth` guard and
//! `oidc_provider.claim_mappings`, through the proxy, with the real kubectl
//! and the bench's Dex.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use integration::{Bench, Persona, Session, fixture, observe};
use serde_json::{Value, json};

/// Wait until `status.upstreamAuth.trust` reads `expected`: a probe round
/// (60 s), then the controller's requeue (60 s).
async fn wait_trust(ns: &str, expected: &str) {
    let mut trust = String::new();
    for _ in 0..90 {
        let out = Bench::get().admin(&[
            "-n",
            ns,
            "get",
            &format!("proxykubeapi/{}", fixture::CLUSTER),
            "-o",
            "jsonpath={.status.upstreamAuth.trust}",
        ]);
        trust = String::from_utf8_lossy(&out.stdout).to_string();
        if trust == expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }
    assert_eq!(trust, expected, "status.upstreamAuth.trust");
}

/// A cluster the proxy closes: the request answers `503` with the proxy's
/// `Status` naming `cause`, and kubectl fails with `ServiceUnavailable`.
/// kubectl cannot print the message itself: the `503` comes during
/// discovery, which client-go reads raw, without decoding the `Status`.
async fn assert_closed(p: &fixture::Proxied, cause: &str) {
    let ns = &p.ns.name;
    let token = p.reader.cluster_token(ns, fixture::CLUSTER).await;
    let response = Bench::get()
        .proxied(
            reqwest::Method::GET,
            ns,
            fixture::CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            &token,
        )
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 503);
    let body: Value = response.json().await.expect("a Kubernetes Status");
    assert!(
        body["message"].as_str().unwrap_or_default().contains(cause),
        "{body}"
    );
    let out = p.reader.kubectl(&["-n", ns, "get", "pods"]).await;
    assert!(
        !out.success() && out.stderr.contains("(ServiceUnavailable)"),
        "kubectl must fail with the 503: {out:?}"
    );
}

/// `kubectl patch --type merge` of a cluster, then wait for the controller
/// to reconcile it: `ready_reason` is the reason `Ready` must end on.
fn patch_cluster(ns: &str, cluster: &str, spec: &Value, ready_reason: &str) {
    let bench = Bench::get();
    let patch = json!({ "spec": spec }).to_string();
    let out = bench.admin(&[
        "-n",
        ns,
        "patch",
        &format!("proxykubeapi/{cluster}"),
        "--type=merge",
        "-p",
        &patch,
    ]);
    assert!(
        out.status.success(),
        "patch: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    fixture::wait(
        ns,
        &[
            &format!("proxykubeapi/{cluster}"),
            &format!(
                "--for=jsonpath={{.status.conditions[?(@.type==\"Ready\")].reason}}={ready_reason}"
            ),
        ],
    );
    if ready_reason == "Reconciled" {
        // The Redis copy the proxy routes from follows the status.
        let generation = bench.admin(&[
            "-n",
            ns,
            "get",
            &format!("proxykubeapi/{cluster}"),
            "-o",
            "jsonpath={.metadata.generation}",
        ]);
        let generation = String::from_utf8_lossy(&generation.stdout).to_string();
        fixture::wait(
            ns,
            &[
                &format!("proxykubeapi/{cluster}"),
                &format!("--for=jsonpath={{.status.observedGeneration}}={generation}"),
            ],
        );
    }
}

/// The `auth_config` of the fixture, validated against Dex's `/userinfo`
/// through `mappings`.
fn oidc_mapped(mappings: &Value) -> Value {
    let bench = Bench::get();
    json!({
        "validate_against": "OidcProvider",
        "oidc_provider": {
            "enabled": true,
            "issuer_url": bench.dex_url(),
            "client_id": "kube_login",
            "extra_scope": "openid profile email groups",
            "claim_mappings": mappings,
        },
    })
}

/// The `user` of the `request` audit lines for `ns/cluster`.
fn audited_users(ns: &str, cluster: &str) -> Vec<String> {
    observe::audit_lines(ns)
        .into_iter()
        .filter(|line| line["event"] == "request" && line["ns"] == ns && line["cluster"] == cluster)
        .filter_map(|line| line["user"].as_str().map(str::to_string))
        .collect()
}

/// Point 1: a cluster in `Passthrough` whose `oidc_provider` maps the
/// username from `email` — kubectl reaches the pods, and the proxy's audit
/// names the user by the mapped claim, with `upstream_mode: Passthrough`.
#[tokio::test]
async fn p1_claim_mappings_name_the_user_in_passthrough() {
    let p = fixture::proxied("rfc0014-p1", json!(null)).await;
    let ns = p.ns.name.clone();
    patch_cluster(
        &ns,
        fixture::CLUSTER,
        &json!({ "auth_config": oidc_mapped(&json!({
            "username": { "claim": "email", "prefix": "" },
            "groups": { "claim": "groups", "prefix": "" },
        })) }),
        "Reconciled",
    );
    let out = p.reader.kubectl(&["-n", &ns, "get", "pods"]).await;
    assert!(out.success(), "kubectl get pods: {out:?}");
    let lines: Vec<Value> = observe::audit_lines(&ns)
        .into_iter()
        .filter(|line| line["event"] == "request" && line["outcome"] == "allowed")
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line["user"] == "reader@it.local" && line["upstream_mode"] == "Passthrough"),
        "no allowed request audited as reader@it.local in Passthrough: {lines:?}"
    );
}

/// Point 2 (refusal): a cluster asking for `Issuer` on an installation
/// whose `allowedModes` does not hold it — kubectl gets the proxy's `503`
/// with its message, and the Condition says why. The message is the
/// proxy's own: nothing was relayed, the apiserver has no such text.
#[tokio::test]
async fn p2_a_mode_outside_allowed_modes_answers_503_and_is_never_relayed() {
    let p = fixture::proxied("rfc0014-p2", json!(null)).await;
    let ns = p.ns.name.clone();
    let bench = Bench::get();
    patch_cluster(
        &ns,
        fixture::CLUSTER,
        &json!({
            "auth_config": oidc_mapped(&json!({ "username": { "claim": "sub", "prefix": "" } })),
            "upstream_auth": {
                "mode": "Issuer",
                "user_prefix": "proxyauth:",
                "group_prefix": "proxyauth:",
                "issuer_key": { "issuer_url": format!("{}/issuer/{ns}/{}", bench.base_url(), fixture::CLUSTER) },
            },
        }),
        "UpstreamModeNotAllowed",
    );
    let reason = bench.admin(&[
        "-n",
        &ns,
        "get",
        &format!("proxykubeapi/{}", fixture::CLUSTER),
        "-o",
        "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}",
    ]);
    assert_eq!(
        String::from_utf8_lossy(&reason.stdout),
        "UpstreamModeNotAllowed"
    );
    let mode = bench.admin(&[
        "-n",
        &ns,
        "get",
        &format!("proxykubeapi/{}", fixture::CLUSTER),
        "-o",
        "jsonpath={.status.upstreamAuth.mode}",
    ]);
    assert_eq!(String::from_utf8_lossy(&mode.stdout), "Issuer");

    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    let response = bench
        .proxied(
            reqwest::Method::GET,
            &ns,
            fixture::CLUSTER,
            &format!("/api/v1/namespaces/{ns}/pods"),
            &token,
        )
        .send()
        .await
        .expect("request");
    assert_eq!(response.status(), 503);
    let body: Value = response.json().await.expect("a Kubernetes Status");
    let message = body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("upstream mode Issuer is not enabled")
            && message.contains("upstreamAuth.allowedModes"),
        "{body}"
    );
    // The refusal is audited, with the mode that was asked for.
    assert!(
        observe::audit_lines(&ns)
            .iter()
            .any(|line| line["event"] == "request"
                && line["status"] == 503
                && line["upstream_mode"] == "Issuer"),
        "no audited 503 for the Issuer cluster"
    );
    // kubectl reads discovery raw: it shows the code, not the message.
    let out = p.reader.kubectl(&["-n", &ns, "get", "pods"]).await;
    assert!(
        !out.success() && out.stderr.contains("(ServiceUnavailable)"),
        "kubectl must fail with the 503: {out:?}"
    );
}

/// Point 15 (refusal): two clusters on the same Dex client, one mapping the
/// username from `email`, the other mapping everyone to `admin` — the same
/// token sent to both, the second within the cache's lifetime, is audited
/// under each cluster's own mapping (decision 38).
#[tokio::test]
async fn p15_a_mapping_from_one_cluster_never_serves_another() {
    let p = fixture::proxied("rfc0014-p15", json!(null)).await;
    let ns = p.ns.name.clone();
    patch_cluster(
        &ns,
        fixture::CLUSTER,
        &json!({ "auth_config": oidc_mapped(&json!({
            "username": { "claim": "email", "prefix": "" },
            "groups": { "claim": "groups", "prefix": "" },
        })) }),
        "Reconciled",
    );
    let bench = Bench::get();
    let mut other: Value = serde_json::from_slice(
        &bench
            .admin(&[
                "-n",
                &ns,
                "get",
                &format!("proxykubeapi/{}", fixture::CLUSTER),
                "-o",
                "json",
            ])
            .stdout,
    )
    .expect("the fixture's cluster");
    other["metadata"] = json!({ "name": "admin-mapped", "namespace": ns });
    other.as_object_mut().expect("object").remove("status");
    other["spec"]["auth_config"] = oidc_mapped(&json!({
        "username": { "expression": "'admin'" },
        "groups": { "claim": "groups", "prefix": "" },
    }));
    bench.apply(&other.to_string());
    fixture::wait(
        &ns,
        &[
            "proxykubeapi/admin-mapped",
            "--for=jsonpath={.status.exposed}=true",
        ],
    );

    // One token, the one kubectl uses for `c`, sent to both clusters.
    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    for cluster in ["admin-mapped", fixture::CLUSTER] {
        let response = bench
            .proxied(
                reqwest::Method::GET,
                &ns,
                cluster,
                &format!("/api/v1/namespaces/{ns}/pods"),
                &token,
            )
            .send()
            .await
            .expect("request");
        assert!(
            response.status().is_success(),
            "{cluster}: {}",
            response.status()
        );
    }
    assert!(
        audited_users(&ns, "admin-mapped")
            .iter()
            .any(|user| user == "admin"),
        "the admin-mapped cluster audits admin"
    );
    let users = audited_users(&ns, fixture::CLUSTER);
    assert!(
        users.iter().any(|user| user == "reader@it.local")
            && !users.iter().any(|user| user == "admin"),
        "the email-mapped cluster never sees the other's user: {users:?}"
    );
}

// Phase 2: FrontProxy, against the bench's apiserver.

/// What the apiserver says the caller is, through the proxy: `kubectl auth
/// whoami -o json`, its username and groups.
async fn whoami(session: &Session) -> (String, Vec<String>) {
    let out = session.kubectl(&["auth", "whoami", "-o", "json"]).await;
    assert!(out.success(), "kubectl auth whoami: {out:?}");
    let review: Value = serde_json::from_str(&out.stdout).expect("SelfSubjectReview JSON");
    let user = &review["status"]["userInfo"];
    let groups = user["groups"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter_map(|g| g.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (
        user["username"].as_str().unwrap_or_default().to_string(),
        groups,
    )
}

/// Point 3: in `FrontProxy`, the apiserver sees the prefixed identity the
/// proxy asserted — a name only the `X-Remote-*` headers can carry, the
/// Dex token relayed would give the bare email — and the RBAC on the
/// prefixed group lets kubectl through. The audit names both identities.
#[tokio::test]
async fn p3_the_apiserver_sees_the_prefixed_identity() {
    let p = fixture::front_proxied("rfc0014-p3").await;
    let ns = p.ns.name.clone();
    let (username, groups) = whoami(&p.reader).await;
    assert_eq!(username, "proxyauth:reader@it.local");
    assert!(
        groups.iter().any(|g| g == "proxyauth:it-readers"),
        "{groups:?}"
    );
    assert!(
        !groups.iter().any(|g| g == "it-readers"),
        "an unprefixed group: {groups:?}"
    );
    let out = p.reader.kubectl(&["-n", &ns, "get", "pods"]).await;
    assert!(out.success(), "kubectl get pods: {out:?}");
    assert!(
        observe::audit_lines(&ns)
            .iter()
            .any(|line| line["event"] == "request"
                && line["upstream_mode"] == "FrontProxy"
                && line["user"] == "reader@it.local"
                && line["upstream_user"] == "proxyauth:reader@it.local"),
        "no request audited with both identities"
    );
    // The probe's success is visible, not only the absence of a failure.
    wait_trust(&ns, "Trusted").await;
}

/// Point 4 (refusal): identity and impersonation headers a client adds
/// never reach the apiserver: it still sees the asserted identity, and the
/// cluster-wide read it would grant is refused.
#[tokio::test]
async fn p4_client_identity_headers_never_reach_the_apiserver() {
    let p = fixture::front_proxied("rfc0014-p4").await;
    let ns = p.ns.name.clone();
    let bench = Bench::get();
    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    let response = bench
        .proxied(
            reqwest::Method::POST,
            &ns,
            fixture::CLUSTER,
            "/apis/authentication.k8s.io/v1/selfsubjectreviews",
            &token,
        )
        .header("X-Remote-User", "system:admin")
        .header("X-Remote-Group", "system:masters")
        .header("Impersonate-User", "admin")
        .json(&json!({ "apiVersion": "authentication.k8s.io/v1", "kind": "SelfSubjectReview" }))
        .send()
        .await
        .expect("SelfSubjectReview");
    assert!(response.status().is_success(), "{}", response.status());
    let review: Value = response.json().await.expect("JSON");
    assert_eq!(
        review["status"]["userInfo"]["username"],
        "proxyauth:reader@it.local"
    );
    let out = p
        .reader
        .kubectl(&["--as", "admin", "get", "secrets", "-A"])
        .await;
    assert!(!out.success(), "a cluster-wide read went through: {out:?}");
    let out = p.reader.kubectl(&["get", "secrets", "-A"]).await;
    assert!(
        !out.success() && out.stderr.contains("Forbidden"),
        "the apiserver refuses the prefixed user a cluster-wide read: {out:?}"
    );
}

/// Point 6: exec through FrontProxy — the upgrade path carries the asserted
/// identity, and the RBAC that allows it names the prefixed group only.
#[tokio::test]
async fn p6_exec_goes_through_front_proxy() {
    let p = fixture::front_proxied("rfc0014-p6").await;
    let ns = p.ns.name.clone();
    let out = p
        .reader
        .kubectl(&[
            "-n",
            &ns,
            "exec",
            "web",
            "-c",
            "app",
            "--",
            "echo",
            "through-front-proxy",
        ])
        .await;
    assert!(out.success(), "kubectl exec: {out:?}");
    assert!(out.stdout.contains("through-front-proxy"), "{out:?}");
}

/// T29 (refusal): the certificate's Secret bound to another target — the
/// cluster is not ready, kubectl gets the proxy's 503 naming the cause, and
/// the certificate is never presented to the cluster's apiserver.
#[tokio::test]
async fn t29_a_certificate_bound_to_another_target_closes_the_cluster() {
    let p = fixture::front_proxied("rfc0014-t29").await;
    let ns = p.ns.name.clone();
    fixture::install_front_proxy_cert(&ns, fixture::CLUSTER, "external:https://elsewhere:6443");
    fixture::wait(
        &ns,
        &[
            &format!("proxykubeapi/{}", fixture::CLUSTER),
            "--for=jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}=ClientCertTargetMismatch",
        ],
    );
    assert_closed(&p, "not ready for upstream mode FrontProxy").await;
}

/// Point 11 (refusal): admission refuses a `client_cert` in FrontProxy.
#[tokio::test]
async fn p11_admission_refuses_a_client_cert_in_front_proxy() {
    let p = fixture::proxied("rfc0014-p11", json!(null)).await;
    let ns = p.ns.name.clone();
    let patch = json!({ "spec": {
        "client_cert": {
            "cert": { "Secret": { "name": "x", "key": "tls.crt" } },
            "key": { "Secret": { "name": "x", "key": "tls.key" } },
        },
        "auth_config": oidc_mapped(&json!({ "username": { "claim": "sub", "prefix": "" } })),
        "upstream_auth": { "mode": "FrontProxy", "user_prefix": "p:", "group_prefix": "p:" },
    } });
    let out = Bench::get().admin(&[
        "-n",
        &ns,
        "patch",
        &format!("proxykubeapi/{}", fixture::CLUSTER),
        "--type=merge",
        "-p",
        &patch.to_string(),
    ]);
    assert!(!out.status.success(), "admission let a client_cert through");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("client_cert is refused in FrontProxy"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Point 9 (refusal): a certificate of a CA the apiserver does not trust as
/// front-proxy — the trust probe gets `401`, the cluster closes with
/// `FrontProxyNotTrusted`, and kubectl gets the proxy's 503.
#[tokio::test]
async fn p9_a_certificate_of_an_untrusted_ca_closes_the_cluster() {
    let p = fixture::front_proxied("rfc0014-p9").await;
    let ns = p.ns.name.clone();
    let dir = tempfile::tempdir().expect("temp dir");
    let run = |args: &[&str]| {
        let out = std::process::Command::new("openssl")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("openssl is on PATH");
        assert!(out.status.success(), "openssl {args:?}: {out:?}");
    };
    run(&[
        "req",
        "-x509",
        "-newkey",
        "ec",
        "-pkeyopt",
        "ec_paramgen_curve:P-256",
        "-nodes",
        "-days",
        "1",
        "-subj",
        "/CN=untrusted-ca",
        "-keyout",
        "ca.key",
        "-out",
        "ca.crt",
    ]);
    run(&[
        "req",
        "-newkey",
        "ec",
        "-pkeyopt",
        "ec_paramgen_curve:P-256",
        "-nodes",
        "-subj",
        "/CN=proxyauth-it",
        "-keyout",
        "client.key",
        "-out",
        "client.csr",
    ]);
    std::fs::write(
        dir.path().join("client.ext"),
        "extendedKeyUsage=clientAuth\n",
    )
    .expect("ext");
    run(&[
        "x509",
        "-req",
        "-in",
        "client.csr",
        "-CA",
        "ca.crt",
        "-CAkey",
        "ca.key",
        "-CAcreateserial",
        "-days",
        "1",
        "-extfile",
        "client.ext",
        "-out",
        "client.crt",
    ]);
    let read = |file: &str| std::fs::read_to_string(dir.path().join(file)).expect("pem");
    let secret = json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "type": "kubernetes.io/tls",
        "metadata": {
            "name": format!("proxyauth-frontproxy-{ns}.{}", fixture::CLUSTER),
            "namespace": integration::bench::STACK_NS,
            "labels": { "app.kubernetes.io/managed-by": "proxyauthk8s" },
            "annotations": { "proxyauth.weebo.si.rs/front-proxy-target": "service:default/kubernetes:443" },
        },
        "stringData": { "tls.crt": read("client.crt"), "tls.key": read("client.key") },
    });
    Bench::get().apply(&secret.to_string());
    // A probe round (60 s), then the controller's requeue (60 s).
    let mut reason = String::new();
    for _ in 0..90 {
        let out = Bench::get().admin(&[
            "-n",
            &ns,
            "get",
            &format!("proxykubeapi/{}", fixture::CLUSTER),
            "-o",
            "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}",
        ]);
        reason = String::from_utf8_lossy(&out.stdout).to_string();
        if reason == "FrontProxyNotTrusted" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }
    assert_eq!(reason, "FrontProxyNotTrusted");
    wait_trust(&ns, "Rejected").await;
    assert_closed(&p, "not ready for upstream mode FrontProxy").await;
}

/// Point 5: Dex hands out `system:masters`; the proxy forwards it prefixed,
/// so the apiserver sees `proxyauth:system:masters`, which grants nothing.
#[tokio::test]
async fn p5_a_system_masters_group_is_forwarded_prefixed_and_grants_nothing() {
    let p = fixture::front_proxied("rfc0014-p5").await;
    let masters = Session::server_login(&Persona::named("masters")).await;
    masters
        .cluster_login(&p.ns.name, fixture::CLUSTER)
        .await
        .expect("cluster login of masters");
    let (username, groups) = whoami(&masters).await;
    assert_eq!(username, "proxyauth:masters@it.local");
    assert!(
        groups.iter().any(|g| g == "proxyauth:system:masters"),
        "{groups:?}"
    );
    assert!(
        !groups.iter().any(|g| g == "system:masters"),
        "an unprefixed system:masters: {groups:?}"
    );
    let out = masters.kubectl(&["get", "nodes"]).await;
    assert!(
        !out.success() && out.stderr.contains("Forbidden"),
        "proxyauth:system:masters must not be admin: {out:?}"
    );
}

/// Point 5 (refusal): a username claim Dex does not send. The proxy refuses
/// the caller itself (`401`, §17.2 deviation 4): an apiserver `401` would
/// have become the `502` of decision 46, so nothing was asserted upstream.
#[tokio::test]
async fn p5_a_missing_username_claim_is_refused_by_the_proxy() {
    let p = fixture::front_proxied("rfc0014-p5b").await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    let patch = json!({ "spec": { "auth_config": { "oidc_provider": { "claim_mappings": {
        "username": { "claim": "not_sent_by_dex", "prefix": "" } } } } } });
    let out = Bench::get().admin(&[
        "-n",
        &ns,
        "patch",
        &format!("proxykubeapi/{}", fixture::CLUSTER),
        "--type=merge",
        "-p",
        &patch.to_string(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // `Ready` stays `Reconciled`: wait for the proxy to read the new mapping.
    let mut status = 0;
    for _ in 0..30 {
        status = Bench::get()
            .proxied(
                reqwest::Method::GET,
                &ns,
                fixture::CLUSTER,
                &format!("/api/v1/namespaces/{ns}/pods"),
                &token,
            )
            .send()
            .await
            .expect("request")
            .status()
            .as_u16();
        assert_ne!(status, 502, "the apiserver was asked");
        if status == 401 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    assert_eq!(status, 401);
}

/// Point 7: `FrontProxy` behind an agent (RFC 0012) — the certificate's TLS
/// runs end to end through the tunnel, and the apiserver sees the prefixed
/// identity; the probe, through the same tunnel, says `Trusted`.
#[tokio::test]
async fn p7_front_proxy_crosses_the_agent_tunnel() {
    let p = fixture::agent_proxied("rfc0014-p7", Value::Null, 1).await;
    let target = format!("agent:{}/{}", p.ns.name, fixture::CLUSTER);
    fixture::use_front_proxy(&p, &target);
    let (username, groups) = whoami(&p.reader).await;
    assert_eq!(username, "proxyauth:reader@it.local");
    assert!(
        groups.iter().any(|g| g == "proxyauth:it-readers"),
        "{groups:?}"
    );
    wait_trust(&p.ns.name, "Trusted").await;
}

/// Point 8 (refusal): straight to the apiserver, without the proxy's
/// certificate, the `X-Remote-*` headers authenticate nobody: `401`.
#[tokio::test]
async fn p8_identity_headers_without_the_certificate_are_refused() {
    let bench = Bench::get();
    let server = bench.admin(&[
        "config",
        "view",
        "--minify",
        "-o",
        "jsonpath={.clusters[0].cluster.server}",
    ]);
    let server = String::from_utf8_lossy(&server.stdout).trim().to_string();
    let ca = bench.admin(&[
        "-n",
        "default",
        "get",
        "configmap",
        "kube-root-ca.crt",
        "-o",
        "jsonpath={.data.ca\\.crt}",
    ]);
    let ca = reqwest::Certificate::from_pem(&ca.stdout).expect("the apiserver's CA");
    let client = reqwest::Client::builder()
        .add_root_certificate(ca)
        .build()
        .expect("client");
    let response = client
        .post(format!(
            "{server}/apis/authentication.k8s.io/v1/selfsubjectreviews"
        ))
        .header("X-Remote-User", "proxyauth:reader@it.local")
        .header("X-Remote-Group", "proxyauth:it-readers")
        .json(&json!({ "apiVersion": "authentication.k8s.io/v1", "kind": "SelfSubjectReview" }))
        .send()
        .await
        .expect("request to the apiserver");
    assert_eq!(response.status(), 401);
}

/// Point 14: the caller's Dex token never reaches the proxy's logs, the
/// audit lines included, after a request through `FrontProxy`.
#[tokio::test]
async fn p14_the_dex_token_never_reaches_the_logs() {
    let p = fixture::front_proxied("rfc0014-p14").await;
    let ns = p.ns.name.clone();
    let token = p.reader.cluster_token(&ns, fixture::CLUSTER).await;
    let out = p.reader.kubectl(&["-n", &ns, "get", "pods"]).await;
    assert!(out.success(), "kubectl get pods: {out:?}");
    // The signature alone is enough to recognise a logged token.
    let signature = token.rsplit('.').next().unwrap_or_default();
    assert!(signature.len() > 16, "not a JWT");
    let logs = Bench::get().admin(&[
        "-n",
        integration::bench::STACK_NS,
        "logs",
        &format!("deployment/{}", integration::bench::BACK_SERVICE),
        "--tail=-1",
    ]);
    assert!(logs.status.success(), "read the back's logs");
    assert!(
        !String::from_utf8_lossy(&logs.stdout).contains(signature),
        "the Dex token is in the back's logs"
    );
}
