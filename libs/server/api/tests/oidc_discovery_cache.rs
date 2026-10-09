//! `DiscoveryCache` measured against a real wiremock provider.
//!
//! The point of the cache is not tidiness — it is how many times the provider
//! is contacted. Resolving one caller used to fetch
//! `/.well-known/openid-configuration` twice: once inside `oidc_core()` to build
//! the client, and again inside `ensure_token_audience()` to look for an
//! introspection endpoint. These tests count the requests wiremock actually
//! received, so they fail if the caching regresses rather than merely if the
//! code stops compiling.

mod harness;

use api::model::user::User;
use common::discovery_cache::DiscoveryCache;
use common::oidc_conf::OidcConf;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A JWT-shaped access token carrying `aud: proxyauthk8s`. `/userinfo` proves
/// the token is valid; the audience check that follows reads this `aud`, so an
/// opaque string would be (correctly) rejected by the default enforce mode.
const TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJhdWQiOiJwcm94eWF1dGhrOHMiLCJzdWIiOiJhbGljZS1zdWIifQ.c2lnbmF0dXJlLW5vdC12ZXJpZmllZC1pbi10aGVzZS10ZXN0cw"; // gitleaks:allow

/// Count the requests wiremock received for `path`.
async fn hits(server: &MockServer, wanted: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("wiremock should record requests")
        .iter()
        .filter(|req| req.url.path() == wanted)
        .count()
}

/// A provider serving discovery + userinfo, and advertising no introspection
/// endpoint — the common case, and the one that used to re-ask every request.
async fn provider() -> MockServer {
    let server = MockServer::start().await;
    let issuer = server.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "userinfo_endpoint": format!("{issuer}/userinfo"),
            "jwks_uri": format!("{issuer}/jwks"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        })))
        .mount(&server)
        .await;

    // Discovery is two round-trips: the well-known document, then the `jwks_uri`
    // it points at. Both must be mounted or `discover_async` fails with a 404 —
    // and both are skipped together on a cache hit.
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [] })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "sub": "alice-sub",
                    "preferred_username": "alice",
                    "email": "alice@example.com",
                    "groups": ["dev"],
                })),
        )
        .mount(&server)
        .await;

    server
}

fn conf(issuer: &str) -> OidcConf {
    OidcConf {
        client_id: "proxyauthk8s".to_string(),
        client_secret: None,
        issuer_url: issuer.to_string(),
        scopes: "openid".to_string(),
        audience: "proxyauthk8s".to_string(),
        accept_authorized_party: false,
        redirect_url: None,
    }
}

/// The headline: repeated logins must not repeatedly re-discover.
#[tokio::test]
async fn discovery_is_fetched_once_across_many_resolutions() {
    let server = provider().await;
    let cache = DiscoveryCache::new();

    for attempt in 1..=5 {
        User::get_user_info_from_oidc_token(TOKEN.to_string(), conf(&server.uri()), &cache)
            .await
            .unwrap_or_else(|e| panic!("resolution {attempt} should succeed: {e}"))
            .expect("a user should be resolved");
    }

    // Two, not one, and deliberately so: `oidc_core()` and the introspection
    // lookup keep separate caches, because the typed metadata has to come from
    // `discover_async` (which performs the issuer check) while the introspection
    // endpoint is not a field that type carries. Both are then cached, so the
    // cost is two fetches per issuer per TTL rather than two per request — these
    // five resolutions would have cost ten without the cache.
    assert_eq!(
        hits(&server, "/.well-known/openid-configuration").await,
        2,
        "five resolutions should have discovered the provider twice, not ten times"
    );
    assert_eq!(
        hits(&server, "/jwks").await,
        1,
        "the jwks leg of discovery should be skipped on a cache hit too"
    );
    assert_eq!(
        hits(&server, "/userinfo").await,
        5,
        "userinfo is per-token and must NOT be cached"
    );
}

/// Both consumers — `oidc_core()` and the introspection lookup — must share one
/// fetch. A single resolution previously cost two.
#[tokio::test]
async fn one_resolution_costs_one_discovery_fetch() {
    let server = provider().await;
    let cache = DiscoveryCache::new();

    User::get_user_info_from_oidc_token(TOKEN.to_string(), conf(&server.uri()), &cache)
        .await
        .expect("resolution should succeed")
        .expect("a user should be resolved");

    assert_eq!(
        hits(&server, "/.well-known/openid-configuration").await,
        2,
        "one resolution costs one fetch per consumer, and no more"
    );
}

/// Each issuer is discovered on its own; one provider's document must never be
/// served for another, or tokens would be validated against the wrong endpoints.
#[tokio::test]
async fn separate_issuers_are_discovered_separately() {
    let first = provider().await;
    let second = provider().await;
    let cache = DiscoveryCache::new();

    for server in [&first, &second] {
        User::get_user_info_from_oidc_token(TOKEN.to_string(), conf(&server.uri()), &cache)
            .await
            .expect("resolution should succeed")
            .expect("a user should be resolved");
    }

    assert_eq!(hits(&first, "/.well-known/openid-configuration").await, 2);
    assert_eq!(hits(&second, "/.well-known/openid-configuration").await, 2);
}

/// A fresh cache must actually fetch — otherwise the tests above would pass on a
/// cache that never contacted anyone.
#[tokio::test]
async fn an_empty_cache_does_fetch() {
    let server = provider().await;

    assert_eq!(hits(&server, "/.well-known/openid-configuration").await, 0);
    User::get_user_info_from_oidc_token(
        TOKEN.to_string(),
        conf(&server.uri()),
        &DiscoveryCache::new(),
    )
    .await
    .expect("resolution should succeed")
    .expect("a user should be resolved");
    assert!(
        hits(&server, "/.well-known/openid-configuration").await > 0,
        "an empty cache must actually contact the provider"
    );
}

/// A provider that cannot be reached must not poison the cache.
///
/// This is the distinction the introspection lookup has to make: it cannot tell
/// "no introspection endpoint" from "could not ask" by return value alone, so a
/// failed fetch must leave the cache untouched. Caching it would silently
/// downgrade audience validation to the JWT-claims fallback for a whole TTL
/// because the provider blipped once.
#[tokio::test]
async fn a_failed_fetch_is_not_cached() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let cache = DiscoveryCache::new();
    // Result is irrelevant here — what matters is what it left behind.
    let _ = conf(&server.uri())
        .ensure_token_audience(TOKEN, &cache)
        .await;

    assert!(
        cache.introspection_endpoint(&server.uri()).await.is_none(),
        "a failed discovery must leave no entry, so the next request retries"
    );
    assert!(
        cache.metadata(&server.uri()).await.is_none(),
        "a failed discovery must not populate the metadata cache either"
    );
}

/// The opposite case, and the one that carries the benefit: a provider that
/// answers "I have no introspection endpoint" is a real answer and is
/// remembered, so the next request does not re-ask.
#[tokio::test]
async fn a_provider_without_introspection_is_only_asked_once() {
    let server = provider().await;
    let cache = DiscoveryCache::new();
    let conf = conf(&server.uri());

    for _ in 1..=3 {
        let _ = conf.ensure_token_audience(TOKEN, &cache).await;
    }

    assert_eq!(
        cache.introspection_endpoint(&server.uri()).await,
        Some(None),
        "the absence should have been cached as a known answer"
    );
    assert_eq!(
        hits(&server, "/.well-known/openid-configuration").await,
        1,
        "three audience checks should have discovered the provider once"
    );
}
