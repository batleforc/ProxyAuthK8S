//! RFC 0014 `Issuer`, end to end in process (RFC 0015 §6.5): the documents
//! the apiserver loads and the JWT the proxy signs, from a key Secret served
//! by a wiremock standing in for the proxy's own apiserver. The k3d suite
//! proves the same against a real apiserver; this one runs on every PR and
//! judges the mutants of `issuer.rs`.
#![expect(
    clippy::expect_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]
#![expect(
    clippy::indexing_slicing,
    reason = "a JSON or byte index out of range is a failed test"
)]

mod harness;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::cluster::redirect::upstream_identity::UpstreamIdentity;
use axum::http::{StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use crd::ProxyKubeApi;
use crd::upstream_auth::issuer_key::{self, IssuerKey, IssuerKeySet};
use crd::upstream_auth::{IssuerKeySettings, UpstreamAuth, UpstreamAuthMode};
use harness::{delete_proxy, memory_state_on_kube, memory_store, proxy_fixture, seed_proxy, test};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISSUER_URL: &str = "https://proxy.example.com/issuer/default/issuing";
const UID: &str = "0a1b2c3d-uid";

fn now() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970");
    i64::try_from(since.as_secs()).expect("seconds fit in i64")
}

fn key(kid: &str, activate_at: i64, retire_at: Option<i64>) -> IssuerKey {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("P-256 key");
    let point = pair.public_key_raw();
    IssuerKey {
        kid: kid.into(),
        pkcs8: STANDARD.encode(pair.serialize_der()),
        x: URL_SAFE_NO_PAD.encode(&point[1..33]),
        y: URL_SAFE_NO_PAD.encode(&point[33..65]),
        created_at: activate_at,
        activate_at,
        retire_at,
    }
}

fn issuing_proxy() -> ProxyKubeApi {
    let mut proxy = proxy_fixture("default", "issuing", "https://cluster.example.com");
    proxy.metadata.uid = Some(UID.into());
    proxy.spec.upstream_auth = Some(UpstreamAuth {
        mode: UpstreamAuthMode::Issuer,
        user_prefix: Some("proxyauth:".into()),
        group_prefix: Some("proxyauth:".into()),
        issuer_key: Some(IssuerKeySettings {
            issuer_url: ISSUER_URL.into(),
            token_ttl_seconds: 300,
            rotation_days: 30,
        }),
    });
    proxy
}

/// The key Secret of `UID`, as the controller writes it, served `times`
/// times at most.
async fn serve_keys(kube: &MockServer, keys: &IssuerKeySet, uid_label: &str, times: u64) {
    let name = format!("proxyauth-issuer-{UID}");
    let secret = json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": name,
            "namespace": "default",
            "labels": issuer_key::labels(uid_label),
        },
        "data": {
            issuer_key::KEYS_KEY: STANDARD.encode(serde_json::to_vec(keys).expect("keys serialise")),
        },
    });
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/namespaces/default/secrets/{name}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(secret))
        .expect(times)
        .mount(kube)
        .await;
}

fn identity(extra: BTreeMap<String, Vec<String>>) -> UpstreamIdentity {
    UpstreamIdentity {
        username: "proxyauth:alice".into(),
        groups: vec!["proxyauth:dev".into()],
        extra,
    }
}

fn verify(jwt: &str, key: &IssuerKey) -> jsonwebtoken::TokenData<Value> {
    let decoding = DecodingKey::from_ec_components(&key.x, &key.y).expect("public key");
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[issuing_proxy().to_identifier()]);
    validation.set_issuer(&[ISSUER_URL]);
    jsonwebtoken::decode::<Value>(jwt, &decoding, &validation).expect("the JWT verifies")
}

/// One read of the Secret serves the documents and every JWT for 30 s; the
/// JWKS publishes the current and the pending key, not the retired one; the
/// JWT is signed by the current key, bounded by its TTL and by the caller's
/// token, and reused until its margin.
#[tokio::test]
async fn the_issuer_serves_its_documents_and_signs_with_the_current_key() {
    let at = now();
    let current = key("current", at - 600, Some(at + 3_600));
    let keys = IssuerKeySet {
        keys: vec![
            key("retired", at - 7_200, Some(at - 60)),
            current.clone(),
            key("pending", at + 300, Some(at + 7_200)),
        ],
    };
    let kube = MockServer::start().await;
    serve_keys(&kube, &keys, UID, 1).await;
    let store = memory_store();
    let proxy = issuing_proxy();
    seed_proxy(&store, &proxy).await;
    let state = memory_state_on_kube(&store, "https://idp.example.com".into(), &kube.uri());
    let app = harness::app(state.clone());

    let get = |document: &'static str| {
        let app = app.clone();
        async move {
            let request = test::TestRequest::get()
                .uri(&format!("/issuer/default/issuing/{document}"))
                .to_request();
            test::call_service(&app, request).await
        }
    };
    let response = get(".well-known/openid-configuration").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().ok()),
        Some(Some("max-age=60"))
    );
    let discovery: Value = test::read_body_json(response).await;
    assert_eq!(discovery["issuer"], ISSUER_URL);
    assert_eq!(discovery["jwks_uri"], format!("{ISSUER_URL}/jwks"));

    let response = get("jwks").await;
    assert_eq!(response.status(), StatusCode::OK);
    let jwks: Value = test::read_body_json(response).await;
    let kids: Vec<&str> = jwks["keys"]
        .as_array()
        .expect("a key list")
        .iter()
        .filter_map(|k| k["kid"].as_str())
        .collect();
    assert_eq!(kids, ["current", "pending"]);

    let jwt = api::issuer::sign(&state, &proxy, &identity(BTreeMap::new()), None)
        .await
        .expect("signed");
    let token = verify(jwt.expose_secret(), &current);
    assert_eq!(token.header.kid.as_deref(), Some("current"));
    let claims = token.claims;
    assert_eq!(claims["sub"], "proxyauth:alice");
    assert_eq!(claims["groups"], json!(["proxyauth:dev"]));
    let iat = claims["iat"].as_i64().expect("iat");
    assert_eq!(claims["exp"].as_i64(), iat.checked_add(300));
    assert_eq!(claims["nbf"].as_i64(), iat.checked_sub(30));
    assert!(claims.get("extra").is_none(), "no extra mapped, none sent");

    // The same identity reuses the JWT.
    let again = api::issuer::sign(&state, &proxy, &identity(BTreeMap::new()), None)
        .await
        .expect("signed");
    assert_eq!(again.expose_secret(), jwt.expose_secret());

    // A caller's token with 120 s left bounds a new one (decision 61).
    let short = api::issuer::sign(
        &state,
        &proxy,
        &identity(BTreeMap::new()),
        Some(Duration::from_secs(120)),
    )
    .await
    .expect("signed");
    let claims = verify(short.expose_secret(), &current).claims;
    let iat = claims["iat"].as_i64().expect("iat");
    assert_eq!(claims["exp"].as_i64(), iat.checked_add(120));

    let extra = BTreeMap::from([("example.com/team".to_string(), vec!["blue".to_string()])]);
    let with_extra = api::issuer::sign(&state, &proxy, &identity(extra), None)
        .await
        .expect("signed");
    let claims = verify(with_extra.expose_secret(), &current).claims;
    assert_eq!(claims["extra"], json!({ "example.com/team": ["blue"] }));

    // Within their 10 s, the documents are served without a lookup: the
    // cluster gone from the store is not seen yet.
    delete_proxy(&store, "default", "issuing").await;
    assert_eq!(get("jwks").await.status(), StatusCode::OK);
    // The Secret was read once (`expect(1)`, checked when `kube` drops).
}

/// No key active yet: nothing is signed, and the error says why.
#[tokio::test]
async fn a_jwt_before_the_first_key_activates_is_refused() {
    let at = now();
    let keys = IssuerKeySet {
        keys: vec![key("pending", at + 300, None)],
    };
    let kube = MockServer::start().await;
    serve_keys(&kube, &keys, UID, 1).await;
    let store = memory_store();
    let proxy = issuing_proxy();
    let state = memory_state_on_kube(&store, "https://idp.example.com".into(), &kube.uri());

    let refused = api::issuer::sign(&state, &proxy, &identity(BTreeMap::new()), None).await;
    assert_eq!(
        refused.err().as_deref(),
        Some("no Issuer key is active yet")
    );
}

/// Decision 43: a Secret of the right name but another cluster's labels is
/// not this cluster's key.
#[tokio::test]
async fn a_key_secret_of_another_cluster_is_rejected() {
    let at = now();
    let keys = IssuerKeySet {
        keys: vec![key("current", at - 600, None)],
    };
    let kube = MockServer::start().await;
    serve_keys(&kube, &keys, "another-uid", 1).await;
    let store = memory_store();
    let proxy = issuing_proxy();
    seed_proxy(&store, &proxy).await;
    let state = memory_state_on_kube(&store, "https://idp.example.com".into(), &kube.uri());

    let refused = api::issuer::sign(&state, &proxy, &identity(BTreeMap::new()), None).await;
    assert!(refused.is_err());
}
