//! Fast-tier integration tests for `GET /{ns}/{cluster}/oauth/authorize`,
//! the entry point of the mediated OAuth Authorization Server flow.

mod harness;

use actix_web::{http::StatusCode, test, web, App};
use api::cluster::auth::oauth::authorize;
use harness::{
    delete_proxy, mount_oidc_provider, oidc_auth_config, oidc_auth_config_with_well_known,
    proxy_fixture, seed_proxy, test_state, try_redis_pool, unique_cluster,
};
use reqwest::Url;
use wiremock::MockServer;

const VALID_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

macro_rules! authorize_app {
    ($state:expr) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state))
                .service(web::scope("/clusters").service(authorize::authorize)),
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

fn location(resp: &actix_web::dev::ServiceResponse) -> Url {
    let raw = resp
        .headers()
        .get("location")
        .expect("a redirect must carry a Location header")
        .to_str()
        .expect("Location must be ASCII");
    Url::parse(raw).expect("Location must be a valid URL")
}

#[actix_web::test]
async fn redirects_to_the_upstream_provider_and_stores_pending_state() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = authorize_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=code&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge={VALID_CHALLENGE}&code_challenge_method=S256"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FOUND);
    let redirect = location(&resp);
    assert_eq!(redirect.origin().unicode_serialization(), idp.uri());
    assert_eq!(redirect.path(), "/authorize");

    let pairs: std::collections::HashMap<_, _> = redirect.query_pairs().collect();
    assert_eq!(pairs.get("client_id").map(|v| v.as_ref()), Some("proxyauthk8s"));
    assert_eq!(
        pairs.get("redirect_uri").map(|v| v.as_ref()),
        Some(format!("https://proxy.example.com/clusters/{ns}/{cluster}/oauth/callback").as_str())
    );
    assert_eq!(pairs.get("response_type").map(|v| v.as_ref()), Some("code"));
    assert_eq!(
        pairs.get("code_challenge_method").map(|v| v.as_ref()),
        Some("S256")
    );
    // The proxy mints its own PKCE pair for the upstream leg — never the
    // external client's challenge.
    assert_ne!(pairs.get("code_challenge").map(|v| v.as_ref()), Some(VALID_CHALLENGE));
    let correlation_id = pairs.get("state").expect("state must be set").clone();

    let mut conn = pool.get().await.expect("redis connection");
    use deadpool_redis::redis::AsyncTypedCommands;
    let stored = conn
        .get(format!("oauth_as_pending:{ns}/{cluster}/{correlation_id}"))
        .await
        .expect("redis get should succeed");
    assert!(stored.is_some(), "pending authorization should be stored");

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn rejects_a_non_loopback_redirect_uri() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = authorize_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=code&client_id=my-cli\
             &redirect_uri=https%3A%2F%2Fevil.example.com%2Fsteal&state=external-state\
             &code_challenge={VALID_CHALLENGE}&code_challenge_method=S256"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redirects_with_an_error_for_an_unsupported_response_type() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = authorize_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=token&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge={VALID_CHALLENGE}&code_challenge_method=S256"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FOUND);
    let redirect = location(&resp);
    let pairs: std::collections::HashMap<_, _> = redirect.query_pairs().collect();
    assert_eq!(
        pairs.get("error").map(|v| v.as_ref()),
        Some("unsupported_response_type")
    );
    assert_eq!(pairs.get("state").map(|v| v.as_ref()), Some("external-state"));

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redirects_with_an_error_for_a_malformed_code_challenge() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = authorize_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=code&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge=too-short&code_challenge_method=S256"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FOUND);
    let redirect = location(&resp);
    let pairs: std::collections::HashMap<_, _> = redirect.query_pairs().collect();
    assert_eq!(pairs.get("error").map(|v| v.as_ref()), Some("invalid_request"));

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn redirects_with_an_error_for_an_unsupported_code_challenge_method() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config_with_well_known(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = authorize_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=code&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge={VALID_CHALLENGE}&code_challenge_method=plain"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::FOUND);
    let redirect = location(&resp);
    let pairs: std::collections::HashMap<_, _> = redirect.query_pairs().collect();
    assert_eq!(pairs.get("error").map(|v| v.as_ref()), Some("invalid_request"));

    delete_proxy(&pool, &ns, &cluster).await;
}

#[actix_web::test]
async fn returns_404_when_discovery_is_not_enabled() {
    let pool = redis_or_skip!();
    let idp = MockServer::start().await;
    mount_oidc_provider(&idp, "alice", &["dev"]).await;

    let (ns, cluster) = unique_cluster();
    let mut proxy = proxy_fixture(&ns, &cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(oidc_auth_config(&idp.uri()));
    seed_proxy(&pool, &proxy).await;

    let app = authorize_app!(test_state(idp.uri()));
    let req = test::TestRequest::get()
        .uri(&format!(
            "/clusters/{ns}/{cluster}/oauth/authorize?response_type=code&client_id=my-cli\
             &redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback&state=external-state\
             &code_challenge={VALID_CHALLENGE}&code_challenge_method=S256"
        ))
        .to_request();
    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    delete_proxy(&pool, &ns, &cluster).await;
}
