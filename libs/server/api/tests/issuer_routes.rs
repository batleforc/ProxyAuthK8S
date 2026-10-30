//! RFC 0014 T15 (decision 44): the apiserver may read `/issuer/*` as often
//! as unknown `kid`s make it, so these routes are never banned nor rate
//! limited; and they hold positive entries only, so a cluster that is not in
//! `Issuer`, or does not exist, answers 404 and enumerates nothing.

mod harness;

use axum::http::StatusCode;
use harness::{memory_state, memory_store, proxy_fixture, seed_proxy, test, unique_cluster};

#[tokio::test]
async fn the_issuer_routes_are_never_throttled_and_answer_404_off_issuer() {
    let store = memory_store();
    let passthrough = unique_cluster();
    seed_proxy(
        &store,
        &proxy_fixture(
            &passthrough.0,
            &passthrough.1,
            "https://cluster.example.com",
        ),
    )
    .await;
    let app = harness::app(memory_state(&store, "https://idp.example.com".to_string()));

    for (ns, cluster) in [
        passthrough,
        ("default".to_string(), "no-such-cluster".to_string()),
    ] {
        for document in [".well-known/openid-configuration", "jwks"] {
            for _ in 0..50 {
                let request = test::TestRequest::get()
                    .uri(&format!("/issuer/{ns}/{cluster}/{document}"))
                    .to_request();
                let status = test::call_service(&app, request).await.status();
                assert_eq!(status, StatusCode::NOT_FOUND, "{ns}/{cluster} {document}");
            }
        }
    }
}
