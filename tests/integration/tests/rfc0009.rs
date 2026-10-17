//! RFC 0009 §6.P: the bench observes what it claims to observe.

use integration::{Bench, Persona, Session, observe};

/// The smoke fixture in a fresh namespace, with the `web` pod running.
fn smoke(test: &str) -> integration::Namespace {
    let bench = Bench::get();
    let ns = bench.namespace(test);
    let busybox = std::fs::read_to_string(bench.root.join(".it/images.lock"))
        .expect("images.lock")
        .lines()
        .find_map(|line| {
            line.strip_prefix("busybox")
                .map(|rest| rest.trim().to_string())
        })
        .expect("busybox in images.lock");
    let yaml = std::fs::read_to_string(bench.root.join("tests/integration/fixtures/smoke.yaml"))
        .expect("smoke fixture")
        .replace("__NS__", &ns.name)
        .replace("__BUSYBOX__", &busybox);
    bench.apply(&yaml);
    // The controller exposes the cluster once it reconciled it (and, after a
    // restart of the back, once it holds the lease): log in only then.
    let out = bench.admin(&[
        "-n",
        &ns.name,
        "wait",
        "proxykubeapi/smoke",
        "--for=jsonpath={.status.exposed}=true",
        "--timeout=120s",
    ]);
    assert!(
        out.status.success(),
        "smoke never exposed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = bench.admin(&[
        "-n",
        &ns.name,
        "wait",
        "pod/web",
        "--for=condition=Ready",
        "--timeout=120s",
    ]);
    assert!(
        out.status.success(),
        "web pod: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    ns
}

/// Point 1: the front answers through the Ingress with its CSP.
#[tokio::test]
async fn p1_front_answers_through_the_ingress_with_a_csp() {
    let bench = Bench::get();
    let response = bench
        .http()
        .get(bench.base_url())
        .send()
        .await
        .expect("GET /");
    assert_eq!(response.status(), 200);
    assert!(
        response.headers().contains_key("content-security-policy"),
        "no CSP header: {:?}",
        response.headers()
    );
}

/// Point 2: a CLI login of `reader`; kubectl reaches the pod through the
/// proxy, and the apiserver itself names the user Dex asserted.
#[tokio::test]
async fn p2_cli_login_reaches_pods_as_the_dex_user() {
    let ns = smoke("rfc0009-p2");
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(&ns.name, "smoke")
        .await
        .expect("cluster login of reader");

    let pods = reader
        .kubectl(&["get", "pods", "-n", &ns.name, "-o", "name"])
        .await;
    assert!(pods.success(), "{pods:?}");
    assert_eq!(pods.stdout.trim(), "pod/web");

    // SelfSubjectReview: answered by the apiserver from the token it
    // validated, not by the proxy.
    let whoami = reader
        .kubectl(&[
            "auth",
            "whoami",
            "-o",
            "jsonpath={.status.userInfo.username}",
        ])
        .await;
    assert!(whoami.success(), "{whoami:?}");
    assert_eq!(whoami.stdout.trim(), "reader@it.local");
}

/// Point 3: refusals. `reader` deleting the pod is refused by the proxy's
/// verb policy (the counter moves by one, the pod stays); `nogroup` cannot
/// even start a login on a cluster restricted to a group.
#[tokio::test]
async fn p3_refusals_come_from_the_proxy() {
    let ns = smoke("rfc0009-p3");
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(&ns.name, "smoke")
        .await
        .expect("cluster login of reader");

    let cluster = format!("{}/smoke", ns.name);
    let labels = [
        ("cluster", cluster.as_str()),
        ("policy", "verb"),
        ("decision", "denied"),
    ];
    let (delta, delete) = observe::metric_delta(
        "proxyauth_policy_decisions_total",
        &labels,
        reader.kubectl(&["delete", "pod", "web", "-n", &ns.name, "--wait=false"]),
    )
    .await;
    assert_eq!(delete.code, Some(1), "{delete:?}");
    assert!(
        delete.stderr.contains("Forbidden") || delete.stderr.contains("forbidden"),
        "{delete:?}"
    );
    assert_eq!(delta, 1.0);
    let still = Bench::get().admin(&["-n", &ns.name, "get", "pod", "web"]);
    assert!(still.status.success(), "the pod was deleted");

    let nogroup = Session::server_login(&Persona::named("nogroup")).await;
    let refused = nogroup.cluster_login(&ns.name, "smoke").await;
    assert!(refused.is_err(), "nogroup logged in: {refused:?}");
}
