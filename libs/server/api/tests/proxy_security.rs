//! Lot 2: per-cluster authorization, the resource allow-list, and the headers
//! the proxy is authoritative for.
//!
//! One wiremock server plays both the OIDC provider and the upstream Kubernetes
//! API, so a request can be authenticated and forwarded in the same scenario.

mod harness;

use actix_web::{App, http::StatusCode, test, web};
use api::cluster::redirect;
use harness::{
    delete_proxy, mount_oidc_provider, oidc_auth_config, proxy_fixture, security_config,
    seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A JWT-shaped access token whose `aud` claim names `oidc_auth_config`'s
/// `client_id` (`proxyauthk8s`). `/userinfo` (mocked below) doesn't check the
/// token value, but `ensure_token_audience` runs on every request after it and
/// fails closed under the default Enforce mode — an opaque token has no `aud`
/// to check, so it would be rejected before ever reaching the group/path
/// checks these tests exist to exercise. Signature is not verified here.
const VALID_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw";

macro_rules! proxy_app {
    ($state:expr_2021) => {
        test::init_service(
            App::new().app_data(web::Data::new($state)).service(
                web::scope("/clusters")
                    .service(redirect::get_redirect)
                    .service(redirect::post_redirect),
            ),
        )
        .await
    };
}

macro_rules! redis_or_skip {
    () => {
        match try_redis_pool().await {
            Some(pool) => pool,
            None => return,
        }
    };
}

/// The proxy answers its own rejections with a Kubernetes `Status` object.
async fn assert_kubernetes_status(resp: actix_web::dev::ServiceResponse, code: u64) {
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["kind"], "Status");
    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["status"], "Failure");
    assert_eq!(body["code"], code);
}

#[actix_web::test]
async fn allows_a_user_inside_the_proxy_group() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["operators"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_user_outside_the_proxy_group() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["viewers"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_kubernetes_status(resp, 403).await;
    // The upstream must not have seen the Kubernetes request at all.
    let kube_calls = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/api/v1/pods")
        .count();
    assert_eq!(kube_calls, 0);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn refuses_a_group_restricted_cluster_when_validation_is_disabled() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // A group is required but nothing resolves a user: the check can never run.
    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.proxy_group = Some("operators".to_string());
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn dashboard_exposure_restricts_the_proxy_too() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["unrelated"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    proxy.spec.expose_via_dashboard = true;
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_path_outside_the_allow_list() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("should not be reached"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![(
        "/api/v1/namespaces/dev/pods/**",
        true,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/kube-system/secrets"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_kubernetes_status(resp, 403).await;
    assert!(
        upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn allows_a_path_inside_the_allow_list() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev/pods/mypod/log"))
        .respond_with(ResponseTemplate::new(200).set_body_string("logs"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![(
        "/api/v1/namespaces/dev/pods/**",
        true,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods/mypod/log"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(test::read_body(resp).await, "logs");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn the_allow_list_matches_on_the_path_not_the_query_string() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/dev/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![(
        "/api/v1/namespaces/dev/pods",
        false,
    )]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/dev/pods?watch=true"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn forwards_the_caller_identity_and_appends_to_the_forwarded_chain() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;
    mount_oidc_provider(&upstream, "alice", &["operators", "dev"]).await;

    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.auth_config = Some(oidc_auth_config(&upstream.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("authorization", format!("Bearer {VALID_TOKEN}")))
        .insert_header(("x-forwarded-for", "203.0.113.7"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    let received = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|r| r.url.path() == "/api/v1/pods")
        .expect("the upstream should have received the request");

    let header = |name: &str| {
        received
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };

    assert_eq!(header("x-forwarded-user").as_deref(), Some("alice"));
    assert_eq!(
        header("x-forwarded-groups").as_deref(),
        Some("operators,dev")
    );
    // The chain the client sent must survive; the peer is appended to it.
    let forwarded_for = header("x-forwarded-for").expect("x-forwarded-for should be present");
    assert!(
        forwarded_for.starts_with("203.0.113.7"),
        "unexpected chain: {forwarded_for}"
    );

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn drops_client_supplied_identity_headers() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    // No token validation, so no identity can be asserted: the forged headers
    // must simply disappear rather than reach the cluster.
    Mock::given(method("GET"))
        .and(path("/api/v1/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pods"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    seed_proxy(&pool, &proxy_fixture(&ns, &cluster, &upstream.uri())).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!("/clusters/{ns}/{cluster}/api/v1/pods"))
        .insert_header(("x-forwarded-user", "cluster-admin"))
        .insert_header(("x-forwarded-groups", "system:masters"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    let received = upstream
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|r| r.url.path() == "/api/v1/pods")
        .expect("the upstream should have received the request");
    assert!(received.headers.get("x-forwarded-user").is_none());
    assert!(received.headers.get("x-forwarded-groups").is_none());

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn an_empty_allow_list_forwards_everything() {
    let pool = redis_or_skip!();
    let upstream = MockServer::start().await;

    Mock::given(method("GET"))
        .and(header_exists("x-forwarded-for"))
        .respond_with(ResponseTemplate::new(200).set_body_string("anything"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(security_config(vec![]));
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/api/v1/namespaces/kube-system/secrets"
        ))
        .insert_header(("x-forwarded-for", "203.0.113.7"))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::OK);

    delete_proxy(&pool, &ns, &cluster).await;
}

/// RFC 0003 §1: one `Path` rule restricting port-forwards to 8080, a `Crd`
/// `Pod` rule without ports, and `extra` rules.
fn port_restricted_config(extra: Vec<(&str, bool)>) -> crd::security::SecurityConfiguration {
    use crd::security::{
        AllowedCrdConfiguration, AllowedPathConfiguration, AllowedPathConfigurationEnum,
        NamespacedAccessConfiguration, NamespacedAccessRuleKind, PortSpec,
    };
    let mut config = security_config(extra);
    config
        .allowed_resources
        .push(AllowedPathConfigurationEnum::Path(
            AllowedPathConfiguration {
                path: "/api/v1/namespaces/dev/pods/*/portforward".to_string(),
                parametised: true,
                allowed_ports: Some(vec![PortSpec("8080".to_string())]),
                verbs: None,
            },
        ));
    for (kind, plural) in [("Pod", "pods"), ("Service", "services")] {
        config
            .allowed_resources
            .push(AllowedPathConfigurationEnum::Crd(AllowedCrdConfiguration {
                group: String::new(),
                version: "v1".to_string(),
                kind: kind.to_string(),
                plural: Some(plural.to_string()),
                namespace: NamespacedAccessConfiguration {
                    enabled: true,
                    rule_kind: NamespacedAccessRuleKind::AllowedNamespaces(vec!["dev".to_string()]),
                },
                namespaced: true,
                allowed_ports: None,
                verbs: None,
            }));
    }
    config
}

/// GET `paths` through a proxy with `config` and the status of each answer;
/// the second value is how many requests reached the upstream.
async fn get_through(
    config: crd::security::SecurityConfiguration,
    paths: &[&str],
) -> Option<(Vec<StatusCode>, usize)> {
    let pool = try_redis_pool().await?;
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("upstream"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(config);
    seed_proxy(&pool, &proxy).await;

    let app = proxy_app!(test_state(upstream.uri()));
    let mut statuses = Vec::new();
    for path in paths {
        let req = test::TestRequest::get()
            .uri(&format!("/clusters/{ns}/{cluster}{path}"))
            .to_request();
        statuses.push(test::call_service(&app, req).await.status());
    }
    let reached = upstream.received_requests().await.unwrap_or_default().len();
    delete_proxy(&pool, &ns, &cluster).await;
    Some((statuses, reached))
}

/// RFC 0003 §6.P points 3 and 4: under a restriction, the `Crd` rules grant no
/// port on `…:port/proxy`, and a named port is refused, without reaching the
/// cluster.
#[actix_web::test]
async fn proxy_subresources_are_held_to_the_port_restriction() {
    let Some((statuses, reached)) = get_through(
        port_restricted_config(vec![]),
        &[
            "/api/v1/namespaces/dev/pods/web:5432/proxy/",
            "/api/v1/namespaces/dev/pods/web:8080/proxy/",
            "/api/v1/namespaces/dev/pods/web%3A5432/proxy/",
            "/api/v1/namespaces/dev/services/web:http/proxy/",
            "/api/v1/namespaces/dev/services/web/proxy/",
        ],
    )
    .await
    else {
        return;
    };
    assert!(
        statuses.iter().all(|s| *s == StatusCode::FORBIDDEN),
        "{statuses:?}"
    );
    assert_eq!(reached, 0, "the cluster was contacted");
}

/// A rule listing the port grants it on the proxy subresource.
#[actix_web::test]
async fn a_rule_with_the_port_grants_the_proxy_subresource() {
    use crd::security::{AllowedPathConfiguration, AllowedPathConfigurationEnum, PortSpec};
    let mut config = port_restricted_config(vec![]);
    config
        .allowed_resources
        .push(AllowedPathConfigurationEnum::Path(
            AllowedPathConfiguration {
                path: "/api/v1/namespaces/dev/services/*/proxy/**".to_string(),
                parametised: true,
                allowed_ports: Some(vec![PortSpec("80".to_string())]),
                verbs: None,
            },
        ));
    let Some((statuses, reached)) = get_through(
        config,
        &[
            "/api/v1/namespaces/dev/services/web:80/proxy/",
            "/api/v1/namespaces/dev/services/web:81/proxy/",
            // Other paths of the `Crd` rules still pass.
            "/api/v1/namespaces/dev/pods/web/log",
        ],
    )
    .await
    else {
        return;
    };
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::FORBIDDEN, StatusCode::OK]
    );
    assert_eq!(reached, 2);
}

/// RFC 0003 §6.P points 5 and 6: `nodes/proxy` is refused under a restriction
/// unless a `Path` rule names `nodes/*/proxy` explicitly.
#[actix_web::test]
async fn node_proxy_needs_an_explicit_rule_under_a_restriction() {
    const NODE: &str = "/api/v1/nodes/n1/proxy/portForward/dev/web?port=8080";
    let Some((broad, reached)) = get_through(
        port_restricted_config(vec![("/api/v1/nodes/**", true)]),
        &[NODE],
    )
    .await
    else {
        return;
    };
    assert_eq!(broad, [StatusCode::FORBIDDEN]);
    assert_eq!(reached, 0, "the kubelet was reached");
    assert!(
        port_restricted_config(vec![("/api/v1/nodes/**", true)])
            .warnings()
            .iter()
            .any(|w| w.contains("nodes/proxy stays denied"))
    );

    let Some((explicit, reached)) = get_through(
        port_restricted_config(vec![("/api/v1/nodes/*/proxy/**", true)]),
        &[NODE],
    )
    .await
    else {
        return;
    };
    assert_eq!(explicit, [StatusCode::OK]);
    assert_eq!(reached, 1);
}

/// RFC 0003 §6.P point 7: without any rule listing ports, nothing changes.
#[actix_web::test]
async fn without_a_restriction_port_paths_pass_as_before() {
    let Some((statuses, reached)) = get_through(
        security_config(vec![
            ("/api/v1/namespaces/dev/**", true),
            ("/api/v1/nodes/**", true),
        ]),
        &[
            "/api/v1/namespaces/dev/pods/web:5432/proxy/",
            "/api/v1/namespaces/dev/services/web:http/proxy/",
            "/api/v1/nodes/n1/proxy/portForward/dev/web?port=8080",
        ],
    )
    .await
    else {
        return;
    };
    assert!(
        statuses.iter().all(|s| *s == StatusCode::OK),
        "{statuses:?}"
    );
    assert_eq!(reached, 3);
}

/// RFC 0006 §1: pods read-only, an exec-free rule set, a ConfigMap rule
/// without verbs, and `extra` rules.
fn verb_restricted_config(extra: Vec<(&str, bool)>) -> crd::security::SecurityConfiguration {
    use crd::security::{
        AllowedCrdConfiguration, AllowedPathConfiguration, AllowedPathConfigurationEnum,
        NamespacedAccessConfiguration, NamespacedAccessRuleKind, Verb,
    };
    let mut config = security_config(extra);
    config
        .allowed_resources
        .push(AllowedPathConfigurationEnum::Path(
            AllowedPathConfiguration {
                path: "/api/v1/namespaces/dev/pods/**".to_string(),
                parametised: true,
                allowed_ports: None,
                verbs: Some(Verb::READ.to_vec()),
            },
        ));
    config
        .allowed_resources
        .push(AllowedPathConfigurationEnum::Crd(AllowedCrdConfiguration {
            group: String::new(),
            version: "v1".to_string(),
            kind: "ConfigMap".to_string(),
            plural: Some("configmaps".to_string()),
            namespace: NamespacedAccessConfiguration {
                enabled: true,
                rule_kind: NamespacedAccessRuleKind::AllowedNamespaces(vec!["dev".to_string()]),
            },
            namespaced: true,
            allowed_ports: None,
            verbs: None,
        }));
    config
}

/// A request for [`send_through`]: method, path and query, extra header.
type TestRequestSpec<'a> = (&'a str, &'a str, Option<(&'a str, &'a str)>);

/// Send `requests` (method, path and query, extra header) through a proxy with
/// `config`: the status of each answer, and what the upstream received.
async fn send_through(
    config: crd::security::SecurityConfiguration,
    requests: &[TestRequestSpec<'_>],
) -> Option<(Vec<StatusCode>, Vec<wiremock::Request>)> {
    let pool = try_redis_pool().await?;
    let upstream = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_string("upstream"))
        .mount(&upstream)
        .await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, &upstream.uri());
    proxy.spec.security_config = Some(config);
    seed_proxy(&pool, &proxy).await;

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(test_state(upstream.uri())))
            .service(
                web::scope("/clusters")
                    .service(redirect::get_redirect)
                    .service(redirect::post_redirect)
                    .service(redirect::put_redirect)
                    .service(redirect::patch_redirect)
                    .service(redirect::delete_redirect),
            ),
    )
    .await;
    let mut statuses = Vec::new();
    for (method, path, header) in requests {
        let mut req = test::TestRequest::default()
            .method(actix_web::http::Method::from_bytes(method.as_bytes()).unwrap())
            .uri(&format!("/clusters/{ns}/{cluster}{path}"));
        if let Some(header) = header {
            req = req.insert_header(*header);
        }
        statuses.push(test::call_service(&app, req.to_request()).await.status());
    }
    let received = upstream.received_requests().await.unwrap_or_default();
    delete_proxy(&pool, &ns, &cluster).await;
    Some((statuses, received))
}

/// RFC 0006 §6.P points 1 to 4, 6 and 8: reads pass, writes, `kubectl debug`,
/// `kubectl run`, writes through a rule without verbs and `OPTIONS` are
/// refused without reaching the cluster.
#[actix_web::test]
async fn verbs_limit_what_a_rule_allows() {
    use StatusCode as S;
    const PODS: &str = "/api/v1/namespaces/dev/pods";
    let Some((statuses, received)) = send_through(
        verb_restricted_config(vec![]),
        &[
            ("GET", PODS, None),
            ("GET", "/api/v1/namespaces/dev/pods/web", None),
            ("GET", "/api/v1/namespaces/dev/pods?watch=1", None),
            ("GET", "/api/v1/namespaces/dev/configmaps/app", None),
            ("DELETE", "/api/v1/namespaces/dev/pods/web", None),
            (
                "PATCH",
                "/api/v1/namespaces/dev/pods/web/ephemeralcontainers",
                None,
            ),
            ("POST", PODS, None),
            ("DELETE", "/api/v1/namespaces/dev/configmaps/app", None),
            // No route serves OPTIONS: refused before the proxy logic.
            ("OPTIONS", PODS, None),
            // T3: a hidden watch is still a read, never a write.
            ("GET", "/api/v1/namespaces/dev/pods?watch=TRUE", None),
        ],
    )
    .await
    else {
        return;
    };
    assert_eq!(
        statuses,
        [
            S::OK,
            S::OK,
            S::OK,
            S::OK,
            S::FORBIDDEN,
            S::FORBIDDEN,
            S::FORBIDDEN,
            S::FORBIDDEN,
            S::NOT_FOUND,
            S::OK
        ]
    );
    assert_eq!(received.len(), 5, "only the reads reached the cluster");
}

/// RFC 0006 §6.P point 9: a method override neither changes the verb nor
/// reaches the cluster.
#[actix_web::test]
async fn a_method_override_is_ignored_and_dropped() {
    let override_get = Some(("X-HTTP-Method-Override", "GET"));
    let Some((statuses, received)) = send_through(
        verb_restricted_config(vec![]),
        &[
            ("POST", "/api/v1/namespaces/dev/pods", override_get),
            (
                "GET",
                "/api/v1/namespaces/dev/pods",
                Some(("X-HTTP-Method-Override", "DELETE")),
            ),
        ],
    )
    .await
    else {
        return;
    };
    assert_eq!(statuses, [StatusCode::FORBIDDEN, StatusCode::OK]);
    assert_eq!(received.len(), 1);
    assert!(
        !received[0].headers.contains_key("x-http-method-override"),
        "the override header was relayed"
    );
}

/// RFC 0006 §6.P point 7 (T8): a verb restriction alone closes `nodes/proxy`,
/// whose `GET` is the kubelet's exec.
#[actix_web::test]
async fn a_verb_restriction_closes_node_proxy() {
    const NODE: &str = "/api/v1/nodes/n1/proxy/exec/dev/web/app?command=sh";
    let Some((statuses, received)) = send_through(
        verb_restricted_config(vec![("/api/v1/nodes/**", true)]),
        &[("GET", NODE, None)],
    )
    .await
    else {
        return;
    };
    assert_eq!(statuses, [StatusCode::FORBIDDEN]);
    assert!(received.is_empty(), "the kubelet was reached");
}

/// T2: a websocket exec is a `GET`, but `create`: a read-only rule refuses it.
#[actix_web::test]
async fn an_exec_is_create_whatever_the_method() {
    let Some((statuses, received)) = send_through(
        verb_restricted_config(vec![]),
        &[(
            "GET",
            "/api/v1/namespaces/dev/pods/web/exec?command=sh",
            None,
        )],
    )
    .await
    else {
        return;
    };
    assert_eq!(statuses, [StatusCode::FORBIDDEN]);
    assert!(received.is_empty());
}
