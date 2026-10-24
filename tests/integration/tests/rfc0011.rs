//! RFC 0011 §6.P, on the bench: what the controller publishes on and around a
//! `ProxyKubeApi` — the `Ready` condition, its Events, its reconcile metric.
//! The CRs are created with the admin kubeconfig: that is the setup, the
//! action under test is the controller's (RFC 0011 §6.P).

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::time::{Duration, Instant};

use integration::{Bench, Namespace, Persona, bench::STACK_NS, observe};
use serde_json::{Value, json};

/// The minimal spec of a cluster in front of the bench's own apiserver.
fn spec(service: Value) -> Value {
    json!({
        "cert": { "ConfigMap": { "name": "kube-root-ca.crt", "key": "ca.crt" } },
        "service": service,
        "expose_via_dashboard": true,
        "dashboard_group": "it-readers",
    })
}

fn bench_apiserver() -> Value {
    json!({ "KubernetesService": { "name": "kubernetes", "namespace": "default", "port": 443 } })
}

/// A Service of `ns` that selects no pod: the probe gets no answer.
fn nowhere(ns: &str) -> Value {
    Bench::get().apply(
        &json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": { "name": "nowhere", "namespace": ns },
            "spec": { "selector": { "app": "nowhere" }, "ports": [{ "port": 443 }] },
        })
        .to_string(),
    );
    json!({ "KubernetesService": { "name": "nowhere", "namespace": ns, "port": 443 } })
}

fn apply_cluster(ns: &str, name: &str, spec: &Value) {
    Bench::get().apply(
        &json!({
            "apiVersion": "weebo.si.rs/v1",
            "kind": "ProxyKubeApi",
            "metadata": { "name": name, "namespace": ns },
            "spec": spec,
        })
        .to_string(),
    );
}

fn admin_json(args: &[&str]) -> Value {
    let mut full = args.to_vec();
    full.extend(["-o", "json"]);
    let out = Bench::get().admin(&full);
    assert!(
        out.status.success(),
        "kubectl {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("kubectl -o json")
}

fn cluster(ns: &str, name: &str) -> Value {
    admin_json(&["-n", ns, "get", "proxykubeapi", name])
}

fn condition(cr: &Value, type_: &str) -> Option<Value> {
    cr["status"]["conditions"]
        .as_array()?
        .iter()
        .find(|c| c["type"] == type_)
        .cloned()
}

/// `kubectl wait` on `proxykubeapi/<name>`; true when it exited 0.
fn wait(ns: &str, name: &str, condition: &str) -> bool {
    Bench::get()
        .admin(&[
            "-n",
            ns,
            "wait",
            &format!("proxykubeapi/{name}"),
            &format!("--for={condition}"),
            "--timeout=90s",
        ])
        .status
        .success()
}

/// The `events.k8s.io` Events about `proxykubeapi/<name>`.
fn events(ns: &str, name: &str) -> Vec<Value> {
    admin_json(&["-n", ns, "get", "events.v1.events.k8s.io"])["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e["regarding"]["kind"] == "ProxyKubeApi" && e["regarding"]["name"] == name)
        .collect()
}

/// Polls `events` until one matches, for at most 60 s.
fn wait_event(ns: &str, name: &str, matches: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(60))
        .expect("a deadline a minute away");
    loop {
        if let Some(event) = events(ns, name).into_iter().find(&matches) {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "no such Event: {:?}",
            events(ns, name)
        );
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn is(event: &Value, type_: &str, reason: &str) -> bool {
    event["type"] == type_ && event["reason"] == reason
}

/// A cluster whose probe answers: `Ready` with `observedGeneration` current.
fn ready_cluster(test: &str) -> (Namespace, &'static str) {
    let ns = Bench::get().namespace(test);
    apply_cluster(&ns.name, "c", &spec(bench_apiserver()));
    assert!(
        wait(&ns.name, "c", "condition=Ready"),
        "{}",
        cluster(&ns.name, "c")
    );
    (ns, "c")
}

/// Point 1: `kubectl wait --for=condition=Ready` exits 0, `Ready` is
/// `True`/`Reconciled`, and `observedGeneration` is the CR's generation.
#[tokio::test]
async fn p1_a_served_cluster_is_ready() {
    let (ns, name) = ready_cluster("rfc0011-p1");
    let cr = cluster(&ns.name, name);
    let ready = condition(&cr, "Ready").expect("a Ready condition");
    assert_eq!(ready["status"], "True");
    assert_eq!(ready["reason"], "Reconciled");
    assert_eq!(
        cr["status"]["observedGeneration"],
        cr["metadata"]["generation"]
    );
}

/// Point 2: a cluster behind a Service with no endpoint is `Ready: False`
/// with `TargetUnreachable`, a `Warning` says so on the CR, and the dashboard
/// lists it as unreachable.
#[tokio::test]
async fn p2_an_unreachable_target_is_refused_and_announced() {
    let bench = Bench::get();
    let ns = bench.namespace("rfc0011-p2");
    apply_cluster(&ns.name, "c", &spec(nowhere(&ns.name)));
    assert!(
        wait(
            &ns.name,
            "c",
            "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}=TargetUnreachable"
        ),
        "{}",
        cluster(&ns.name, "c")
    );
    let ready = condition(&cluster(&ns.name, "c"), "Ready").expect("a Ready condition");
    assert_eq!(ready["status"], "False");
    wait_event(&ns.name, "c", |e| is(e, "Warning", "TargetUnreachable"));

    let token = Persona::named("reader").dashboard_token().await;
    let body: Value = bench
        .http()
        .get(format!("{}/api/v1/clusters", bench.base_url()))
        .bearer_auth(token)
        .send()
        .await
        .expect("list clusters")
        .json()
        .await
        .expect("clusters JSON");
    let listed = body["clusters"]
        .as_array()
        .expect("clusters")
        .iter()
        .find(|c| c["namespace"] == ns.name.as_str() && c["name"] == "c")
        .cloned()
        .expect("the cluster is listed");
    assert_eq!(listed["is_reachable"], false);
}

/// Point 3: a spec `validate()` refuses is `Ready: False`/`InvalidSpec`,
/// without `observedGeneration`, with a `Warning`.
#[tokio::test]
async fn p3_an_invalid_spec_is_refused_without_an_observed_generation() {
    let ns = Bench::get().namespace("rfc0011-p3");
    let mut invalid = spec(bench_apiserver());
    invalid["auth_config"] = admitted_but_invalid_auth_config();
    apply_cluster(&ns.name, "c", &invalid);
    assert!(
        wait(
            &ns.name,
            "c",
            "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}=InvalidSpec"
        ),
        "{}",
        cluster(&ns.name, "c")
    );
    let cr = cluster(&ns.name, "c");
    assert!(cr["status"]["observedGeneration"].is_null(), "{cr}");
    wait_event(&ns.name, "c", |e| is(e, "Warning", "InvalidSpec"));
}

/// An `auth_config` the CRD admits and `validate()` refuses: an `extra`
/// claim mapping without `value_expression` (no CEL rule on `ExtraMapping`,
/// `claim_mappings.rs:136`).
fn admitted_but_invalid_auth_config() -> Value {
    json!({
        "oidc_provider": {},
        "jwt": [{
            "issuer": { "url": "https://issuer.example.com", "audiences": ["proxy"] },
            "claim_mappings": {
                "username": { "claim": "sub", "prefix": "" },
                "extra": [{ "key": "example.com/team" }],
            },
        }],
    })
}

/// Point 4: the cluster of point 2, pointed back at a target that answers,
/// turns `Ready` again: a `Normal` `Reconciled`, a new `lastTransitionTime`,
/// and `observedGeneration` following the new generation.
#[tokio::test]
async fn p4_a_fixed_cluster_turns_ready_again() {
    let ns = Bench::get().namespace("rfc0011-p4");
    apply_cluster(&ns.name, "c", &spec(nowhere(&ns.name)));
    assert!(wait(
        &ns.name,
        "c",
        "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}=TargetUnreachable"
    ));
    let failed = condition(&cluster(&ns.name, "c"), "Ready").expect("a Ready condition");

    // `lastTransitionTime` has a one-second resolution: fix it a second later
    // so a moved timestamp is visible.
    std::thread::sleep(Duration::from_secs(2));
    apply_cluster(&ns.name, "c", &spec(bench_apiserver()));
    assert!(wait(&ns.name, "c", "condition=Ready"));
    let cr = cluster(&ns.name, "c");
    let ready = condition(&cr, "Ready").expect("a Ready condition");
    assert_ne!(ready["lastTransitionTime"], failed["lastTransitionTime"]);
    assert_eq!(
        cr["status"]["observedGeneration"],
        cr["metadata"]["generation"]
    );
    wait_event(&ns.name, "c", |e| is(e, "Normal", "Reconciled"));
}

/// Point 5: a rule a restriction changes raises `RulesRestricted` next to a
/// `Ready: True`, `status.warnings` unchanged; the rule removed, both go away
/// and a second `Normal` `RulesRestricted` is published. kube's `Recorder`
/// keys a series without the note, so that second one only bumps the
/// series of the first (`series.count`), its note stays the first's (RFC 0011
/// §17).
#[tokio::test]
async fn p5_an_advisory_comes_and_goes() {
    let ns = Bench::get().namespace("rfc0011-p5");
    let mut restricted = spec(bench_apiserver());
    // Under a verb restriction, a rule on nodes/ is warned that nodes/proxy
    // stays denied.
    restricted["security_config"] = json!({
        "enabled": true,
        "allowed_resources": [
            { "Path": { "path": "/api/v1/nodes" } },
            { "Path": { "path": "/apis", "verbs": ["get"] } },
        ],
    });
    apply_cluster(&ns.name, "c", &restricted);
    assert!(wait(
        &ns.name,
        "c",
        "jsonpath={.status.conditions[?(@.type==\"RulesRestricted\")].status}=True"
    ));
    let cr = cluster(&ns.name, "c");
    assert_eq!(condition(&cr, "Ready").expect("Ready")["status"], "True");
    assert!(
        cr["status"]["warnings"]
            .as_array()
            .is_some_and(|w| !w.is_empty()),
        "{cr}"
    );

    apply_cluster(&ns.name, "c", &spec(bench_apiserver()));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let cr = cluster(&ns.name, "c");
        if condition(&cr, "RulesRestricted").is_none() {
            assert!(cr["status"]["warnings"].is_null(), "{cr}");
            break;
        }
        assert!(Instant::now() < deadline, "RulesRestricted stayed: {cr}");
        std::thread::sleep(Duration::from_secs(2));
    }
    wait_event(&ns.name, "c", |e| {
        is(e, "Normal", "RulesRestricted") && e["series"]["count"].as_i64() >= Some(2)
    });
}

/// Point 6: a reconcile of a healthy cluster writes nothing: after an
/// annotation triggers one, `resourceVersion` stays put for 90 s and no Event
/// appears (decision 17).
#[tokio::test]
async fn p6_a_healthy_reconcile_writes_nothing() {
    let (ns, name) = ready_cluster("rfc0011-p6");
    let before_events = events(&ns.name, name).len();
    let reconciled = [("kind", "ProxyKubeApi"), ("reason", "Reconciled")];
    let (delta, version) = observe::metric_delta("proxyauth_reconcile_total", &reconciled, async {
        let out = Bench::get().admin(&[
            "-n",
            &ns.name,
            "annotate",
            "--overwrite",
            &format!("proxykubeapi/{name}"),
            "it.weebo.si.rs/poke=1",
            "-o",
            "jsonpath={.metadata.resourceVersion}",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let version = String::from_utf8_lossy(&out.stdout).into_owned();
        tokio::time::sleep(Duration::from_secs(90)).await;
        version
    })
    .await;
    assert!(delta >= 1.0, "the annotation did not trigger a reconcile");
    let cr = cluster(&ns.name, name);
    assert_eq!(cr["metadata"]["resourceVersion"], version.as_str(), "{cr}");
    assert_eq!(events(&ns.name, name).len(), before_events);
}

/// Point 7: the Events of a failing cluster are reported by the pod holding
/// the lease. The bench runs one replica (RFC 0009, decision 33): that a
/// follower writes nothing is the envtest `a_follower_does_not_touch_the_resource`.
#[tokio::test]
async fn p7_only_the_leader_reports() {
    let ns = Bench::get().namespace("rfc0011-p7");
    apply_cluster(&ns.name, "c", &spec(nowhere(&ns.name)));
    let event = wait_event(&ns.name, "c", |e| is(e, "Warning", "TargetUnreachable"));
    let lease = admin_json(&[
        "-n",
        STACK_NS,
        "get",
        "lease",
        "proxy-auth-k8s-leader-election",
    ]);
    assert_eq!(event["reportingInstance"], lease["spec"]["holderIdentity"]);
    assert_eq!(event["reportingController"], "proxy-kube-api-controller");
}

/// Point 8: `proxyauth_reconcile_total` counts a failing reconcile under its
/// reason.
#[tokio::test]
async fn p8_the_reconcile_metric_counts_the_reason() {
    let ns = Bench::get().namespace("rfc0011-p8");
    let labels = [
        ("kind", "ProxyKubeApi"),
        ("result", "error"),
        ("reason", "TargetUnreachable"),
    ];
    let (delta, ()) = observe::metric_delta("proxyauth_reconcile_total", &labels, async {
        apply_cluster(&ns.name, "c", &spec(nowhere(&ns.name)));
        assert!(wait(
            &ns.name,
            "c",
            "jsonpath={.status.conditions[?(@.type==\"Ready\")].reason}=TargetUnreachable"
        ));
    })
    .await;
    assert!(delta >= 1.0);
}

/// T3: the controller may create and patch Events, nothing more.
#[tokio::test]
async fn t3_the_events_grant_is_create_and_patch_only() {
    let roles = admin_json(&[
        "get",
        "clusterroles",
        "-l",
        "app=proxyauth-proxyauthk8s-operator",
    ]);
    let rules: Vec<Value> = roles["items"]
        .as_array()
        .expect("items")
        .iter()
        .flat_map(|role| role["rules"].as_array().cloned().unwrap_or_default())
        .filter(|rule| {
            rule["resources"]
                .as_array()
                .is_some_and(|r| r.contains(&json!("events")))
        })
        .collect();
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_eq!(rules[0]["apiGroups"], json!(["events.k8s.io"]));
    assert_eq!(rules[0]["verbs"], json!(["create", "patch"]));
}

/// Point 11: `proxyauth_leader` is 1 on the back pod named by the lease's
/// `holderIdentity` (the bench runs one replica, RFC 0009 decision 33).
#[tokio::test]
async fn p11_the_leader_reports_itself() {
    let lease = admin_json(&[
        "-n",
        STACK_NS,
        "get",
        "lease",
        "proxy-auth-k8s-leader-election",
    ]);
    let pods = admin_json(&[
        "-n",
        STACK_NS,
        "get",
        "pods",
        "-l",
        "app=proxyauth-proxyauthk8s-back",
    ]);
    let names: Vec<&str> = pods["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|pod| pod["metadata"]["name"].as_str())
        .collect();
    let holder = lease["spec"]["holderIdentity"].as_str().expect("a holder");
    assert!(
        names.contains(&holder),
        "{holder} is not a back pod: {names:?}"
    );
    assert_eq!(
        observe::metric_value(&observe::metrics(), "proxyauth_leader", &[]),
        1.0
    );
}
