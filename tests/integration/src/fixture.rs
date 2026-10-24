//! A proxied cluster for one test: a `ProxyKubeApi` named `c` with the
//! test's `security_config`, in the test's namespace, in front of the bench's
//! own apiserver; the `web` pod (`app` on 8080, `sidecar` on 9090), a `web`
//! Service (`http` → 8080) and a `app` ConfigMap; and the apiserver RBAC that
//! lets `it-readers` do anything in the namespace, so that every refusal a
//! test sees can only come from the proxy.

use serde_json::{Value, json};

use crate::{Bench, Namespace, Persona, Session};

/// The name of the fixture's `ProxyKubeApi`.
pub const CLUSTER: &str = "c";

pub struct Proxied {
    pub ns: Namespace,
    /// `reader`, logged in to the cluster through the SSO flow.
    pub reader: Session,
}

/// Path rules for the discovery requests kubectl makes before anything else.
/// Without `verbs`: under a verb restriction they grant reads only.
pub fn discovery_rules() -> Vec<Value> {
    ["/api", "/apis", "/version"]
        .into_iter()
        .map(|path| json!({ "Path": { "path": path } }))
        .collect()
}

/// The fixture, applied and ready: `web` running, the cluster exposed, and
/// `reader` logged in. `security` is the `security_config` of the cluster.
pub async fn proxied(test: &str, security: Value) -> Proxied {
    let service = json!({ "KubernetesService": {
        "name": "kubernetes", "namespace": "default", "port": 443 } });
    proxied_through(test, service, security, |_| ()).await
}

/// The release name of a test's agent (RFC 0012): its ServiceAccount is
/// `agent-proxyauthk8s-agent`, the subject its CR names.
pub const AGENT_RELEASE: &str = "agent";

/// The `service: Agent` of a test's cluster in `ns`: the bench's apiserver
/// reached through an agent of `ns`, enrolled with its projected token and
/// checked against the bench's own JWKS, inline.
pub fn agent_service(ns: &str) -> Value {
    let jwks = Bench::get().admin(&["get", "--raw", "/openid/v1/jwks"]);
    assert!(jwks.status.success(), "read the bench's JWKS");
    json!({ "Agent": {
        "bootstrap": { "ServiceAccountToken": {
            "subject": format!(
                "system:serviceaccount:{ns}:{AGENT_RELEASE}-proxyauthk8s-agent"),
            "jwks": String::from_utf8_lossy(&jwks.stdout).trim(),
        } },
    } })
}

/// `helm upgrade --install` of the agent chart in `ns`, for `cluster` (its
/// `namespace/name`), with `replicas` replicas and `extra` `--set`s.
pub fn install_agent(ns: &str, cluster: &str, replicas: u32, extra: &[&str]) {
    let bench = Bench::get();
    let cluster = format!("proxy.cluster={cluster}");
    let replicas = format!("replicaCount={replicas}");
    let url = format!(
        "proxy.url=wss://agent.k8s.localhost:{}/agent/v1/connect",
        8443
    );
    let ca = format!("proxy.caBundle={}", bench.ca_path().display());
    let mut args = vec![
        "upgrade",
        "--install",
        AGENT_RELEASE,
        "deploy/chart-agent",
        "-n",
        ns,
        "--set",
        &cluster,
        "--set",
        &replicas,
        "--set",
        &url,
        "--set-file",
        &ca,
        "--set",
        "image.repository=localhost/proxyauthk8s/agent",
        "--set",
        "image.tag=it",
        "--set",
        "image.pullPolicy=Never",
        "--set",
        "podDisruptionBudget.enabled=false",
    ];
    for set in extra {
        args.extend(["--set", set]);
    }
    let out = bench.helm(&args);
    assert!(
        out.status.success(),
        "helm install the agent: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The fixture in front of an agent (RFC 0012 §6.P): the cluster `c` is
/// `service: Agent`, its `replicas` agents run in the test's namespace, and
/// it is exposed once one of them holds a tunnel.
pub async fn agent_proxied(test: &str, security: Value, replicas: u32) -> Proxied {
    let ns = crate::bench::namespace_name(test);
    proxied_through(test, agent_service(&ns), security, |ns| {
        install_agent(ns, &format!("{ns}/{CLUSTER}"), replicas, &[]);
        wait(
            ns,
            &[
                "deployment/agent-proxyauthk8s-agent",
                "--for=condition=Available",
            ],
        );
    })
    .await
}

/// [`proxied`] with `service`; `before_ready` runs once everything is
/// applied, before waiting for the cluster to be exposed.
async fn proxied_through(
    test: &str,
    service: Value,
    security: Value,
    before_ready: impl FnOnce(&str),
) -> Proxied {
    let bench = Bench::get();
    let ns = bench.namespace(test);
    let busybox = image("busybox");
    let items = json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            {
                "apiVersion": "weebo.si.rs/v1",
                "kind": "ProxyKubeApi",
                "metadata": { "name": CLUSTER, "namespace": ns.name },
                "spec": {
                    "cert": { "ConfigMap": { "name": "kube-root-ca.crt", "key": "ca.crt" } },
                    "service": service,
                    "expose_via_dashboard": true,
                    "dashboard_group": "it-readers",
                    "proxy_group": "it-readers",
                    "auth_config": { "oidc_provider": {
                        "enabled": true,
                        "issuer_url": bench.dex_url(),
                        "client_id": "kube_login",
                        "extra_scope": "openid profile email groups",
                    } },
                    "security_config": security,
                },
            },
            {
                "apiVersion": "v1",
                "kind": "Pod",
                "metadata": { "name": "web", "namespace": ns.name, "labels": { "app": "web" } },
                "spec": { "containers": [
                    { "name": "app", "image": busybox,
                      "command": ["httpd", "-f", "-p", "8080"] },
                    { "name": "sidecar", "image": busybox,
                      "command": ["httpd", "-f", "-p", "9090"] },
                ] },
            },
            {
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": { "name": "web", "namespace": ns.name },
                "spec": {
                    "selector": { "app": "web" },
                    "ports": [{ "name": "http", "port": 80, "targetPort": 8080 }],
                },
            },
            {
                "apiVersion": "v1",
                "kind": "ConfigMap",
                "metadata": { "name": "app", "namespace": ns.name },
                "data": { "key": "value" },
            },
            {
                "apiVersion": "rbac.authorization.k8s.io/v1",
                "kind": "RoleBinding",
                "metadata": { "name": "it-readers-admin", "namespace": ns.name },
                "roleRef": { "apiGroup": "rbac.authorization.k8s.io",
                             "kind": "ClusterRole", "name": "admin" },
                "subjects": [{ "apiGroup": "rbac.authorization.k8s.io",
                               "kind": "Group", "name": "it-readers" }],
            },
        ],
    });
    bench.apply(&items.to_string());
    wait(&ns.name, &["pod/web", "--for=condition=Ready"]);
    before_ready(&ns.name);
    // Listed by the API once the controller reconciled it.
    wait(
        &ns.name,
        &[
            &format!("proxykubeapi/{CLUSTER}"),
            "--for=jsonpath={.status.exposed}=true",
        ],
    );
    let reader = Session::server_login(&Persona::named("reader")).await;
    reader
        .cluster_login(&ns.name, CLUSTER)
        .await
        .expect("cluster login of reader");
    Proxied { ns, reader }
}

/// Let `it-readers` use `nodes/proxy` in the apiserver, so a request the
/// proxy lets through is answered by the kubelet instead of refused by RBAC.
pub fn allow_nodes_proxy() {
    let rbac = json!({
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            {
                "apiVersion": "rbac.authorization.k8s.io/v1",
                "kind": "ClusterRole",
                "metadata": { "name": "it-nodes-proxy" },
                "rules": [{ "apiGroups": [""], "resources": ["nodes/proxy"],
                            "verbs": ["get", "create"] }],
            },
            {
                "apiVersion": "rbac.authorization.k8s.io/v1",
                "kind": "ClusterRoleBinding",
                "metadata": { "name": "it-readers-nodes-proxy" },
                "roleRef": { "apiGroup": "rbac.authorization.k8s.io",
                             "kind": "ClusterRole", "name": "it-nodes-proxy" },
                "subjects": [{ "apiGroup": "rbac.authorization.k8s.io",
                               "kind": "Group", "name": "it-readers" }],
            },
        ],
    });
    Bench::get().apply(&rbac.to_string());
}

/// The name of the bench's only node.
pub fn node() -> String {
    let out = Bench::get().admin(&["get", "nodes", "-o", "jsonpath={.items[0].metadata.name}"]);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// An image reference from `.it/images.lock`.
pub fn image(name: &str) -> String {
    std::fs::read_to_string(Bench::get().root.join(".it/images.lock"))
        .expect("read .it/images.lock")
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(key, _)| *key == name)
        .map(|(_, reference)| reference.trim().to_string())
        .unwrap_or_else(|| panic!("no image {name:?} in .it/images.lock"))
}

fn wait(ns: &str, args: &[&str]) {
    let mut full = vec!["-n", ns, "wait"];
    full.extend_from_slice(args);
    full.push("--timeout=120s");
    let out = Bench::get().admin(&full);
    assert!(
        out.status.success(),
        "kubectl wait {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
