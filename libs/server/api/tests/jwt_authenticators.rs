//! `validate_against: JwtAuthenticators` — local JWT validation, end to end.
//!
//! A real RS256 token signed by the harness key, verified against a real JWKS
//! served by wiremock, through the same `User::get_user_info_with_proxy` entry
//! point the request path uses.
//!
//! Deliberately built on [`harness::unreachable_state`]: local validation must
//! not touch Redis, the target cluster, or `/userinfo`, and a state whose Redis
//! cannot be reached proves it rather than asserting it in a comment.
//!
//! `issuer.url` is `http://` here because wiremock does not serve TLS. That is
//! only reachable in a test: admission rejects a non-https issuer (see
//! `Issuer::validate` and its CEL rule), which — like the apiserver — is
//! configuration validation, not something the token path re-checks.

mod harness;

use api::model::user::{User, UserAuthError};
use crd::ProxyKubeApi;
use crd::authentication_configuration::{
    AuthenticationConfiguration, ClaimMappings, ClaimValidationRule, ExtraMapping, Issuer,
    JWTAuthenticator, OidcProvider, PrefixedClaimOrExpression, UserValidationRule, ValidateAgainst,
};
use harness::{mount_jwks_issuer, proxy_fixture, sign_claims, unique_cluster, unreachable_state};
use serde_json::json;
use wiremock::MockServer;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after the epoch")
        .as_secs()
}

fn claim_mapping(claim: &str, prefix: Option<&str>) -> PrefixedClaimOrExpression {
    PrefixedClaimOrExpression {
        prefix: prefix.map(ToString::to_string),
        claim: Some(claim.to_string()),
        expression: None,
    }
}

fn authenticator(issuer_url: &str, audience: &str) -> JWTAuthenticator {
    JWTAuthenticator {
        issuer: Issuer {
            url: issuer_url.to_string(),
            discovery_url: None,
            certificate_authority: None,
            audiences: vec![audience.to_string()],
            audience_match_policy:
                crd::authentication_configuration::AudienceMatchPolicyType::MatchAny,
            egress_selector: crd::authentication_configuration::EgressSelectorType::ControlPlane,
        },
        claim_validation_rules: Vec::new(),
        claim_mappings: ClaimMappings {
            username: Some(claim_mapping("sub", Some("oidc:"))),
            groups: Some(claim_mapping("groups", Some(""))),
            uid: None,
            extra: Vec::new(),
        },
        user_validation_rules: Vec::new(),
    }
}

/// A proxy whose auth config validates locally against `authenticators`.
fn jwt_proxy(ns: &str, cluster: &str, authenticators: Vec<JWTAuthenticator>) -> ProxyKubeApi {
    let mut proxy = proxy_fixture(ns, cluster, "https://cluster.example.com:6443");
    proxy.spec.auth_config = Some(AuthenticationConfiguration {
        jwt: authenticators,
        oidc_provider: OidcProvider {
            enabled: false,
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: None,
            extra_scope: String::new(),
            audience: String::new(),
            accept_authorized_party: false,
            expose_oauth_authorization_server: false,
            config_from: None,
        },
        disable_validation: false,
        validate_against: ValidateAgainst::JwtAuthenticators,
    });
    proxy
}

async fn issuer_server() -> MockServer {
    let server = MockServer::start().await;
    mount_jwks_issuer(&server).await;
    server
}

/// Resolve a token through the same entry point the request path uses.
async fn resolve(proxy: &ProxyKubeApi, token: &str) -> Result<Option<User>, UserAuthError> {
    let state = unreachable_state("http://127.0.0.1:1".to_string());
    User::get_user_info_with_proxy(state, proxy.clone(), token.to_string()).await
}

#[tokio::test]
async fn a_valid_token_maps_to_a_user() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );

    let token = sign_claims(&json!({
        "iss": server.uri(),
        "sub": "1234",
        "aud": "proxyauthk8s",
        "groups": ["dev", "ops"],
        "iat": now(),
        "exp": now() + 300,
    }));

    let user = resolve(&proxy, &token)
        .await
        .expect("a valid token should be accepted")
        .expect("a user should be returned");

    assert_eq!(user.username, "oidc:1234");
    assert_eq!(user.groups, vec!["dev", "ops"]);
}

/// The whole point of local validation: no Redis, no target cluster, no
/// `/userinfo` round trip. `unreachable_state` makes that observable — if the
/// path reached Redis or the cluster, this could not pass.
#[tokio::test]
async fn validation_needs_neither_redis_nor_the_target_cluster() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    // The upstream is a black hole; only the issuer is reachable.
    let mut proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );
    proxy.spec.service = crd::service::Service::ExternalService {
        url: "https://127.0.0.1:1".to_string(),
    };

    let token = sign_claims(&json!({
        "iss": server.uri(),
        "sub": "1234",
        "aud": "proxyauthk8s",
        "iat": now(),
        "exp": now() + 300,
    }));

    assert!(resolve(&proxy, &token).await.is_ok());
}

#[tokio::test]
async fn an_expired_token_is_refused() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );

    let token = sign_claims(&json!({
        "iss": server.uri(),
        "sub": "1234",
        "aud": "proxyauthk8s",
        "iat": now() - 600,
        "exp": now() - 300,
    }));

    let err = resolve(&proxy, &token).await.unwrap_err();
    assert!(
        matches!(
            err,
            UserAuthError::JwtValidation(jwt_validator::JwtValidationError::Rejected(_))
        ),
        "unexpected error: {err}"
    );
}

/// The audience-confusion hole: a token minted by the same issuer for a
/// different application must not authenticate here.
#[tokio::test]
async fn a_token_for_another_audience_is_refused() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );

    let token = sign_claims(&json!({
        "iss": server.uri(),
        "sub": "1234",
        "aud": "some-other-application",
        "iat": now(),
        "exp": now() + 300,
    }));

    let err = resolve(&proxy, &token).await.unwrap_err();
    assert!(
        matches!(
            err,
            UserAuthError::JwtValidation(jwt_validator::JwtValidationError::Rejected(_))
        ),
        "unexpected error: {err}"
    );
}

/// A token whose `iss` names nobody we trust never even reaches a key lookup.
#[tokio::test]
async fn a_token_from_an_unconfigured_issuer_is_refused() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );

    let token = sign_claims(&json!({
        "iss": "https://evil.example.com",
        "sub": "1234",
        "aud": "proxyauthk8s",
        "iat": now(),
        "exp": now() + 300,
    }));

    let err = resolve(&proxy, &token).await.unwrap_err();
    assert!(
        matches!(
            err,
            UserAuthError::JwtValidation(jwt_validator::JwtValidationError::UnknownIssuer { .. })
        ),
        "unexpected error: {err}"
    );
}

/// Signed with a key the issuer does not publish. This is the check everything
/// else rests on: without it, every rule below is decoration.
#[tokio::test]
async fn a_token_signed_by_the_wrong_key_is_refused() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );

    let token = sign_claims(&json!({
        "iss": server.uri(),
        "sub": "1234",
        "aud": "proxyauthk8s",
        "iat": now(),
        "exp": now() + 300,
    }));
    // Keep the header and payload, corrupt only the signature.
    let mut parts: Vec<&str> = token.split('.').collect();
    parts.pop();
    let forged = format!("{}.{}", parts.join("."), "Zm9yZ2Vk");

    let err = resolve(&proxy, &forged).await.unwrap_err();
    assert!(
        matches!(
            err,
            UserAuthError::JwtValidation(jwt_validator::JwtValidationError::Rejected(_))
        ),
        "unexpected error: {err}"
    );
}

/// An `alg: none` token with no signature at all — the oldest JWT forgery there
/// is. `jsonwebtoken` has no `none` variant, so it cannot even be named; the
/// header is refused as unreadable before any key is looked up.
#[tokio::test]
async fn an_unsigned_token_is_refused() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&server.uri(), "proxyauthk8s")],
    );

    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = b64.encode(json!({ "alg": "none", "typ": "JWT" }).to_string());
    let payload = b64.encode(
        json!({
            "iss": server.uri(),
            "sub": "1234",
            "aud": "proxyauthk8s",
            "exp": now() + 300,
        })
        .to_string(),
    );
    let unsigned = format!("{header}.{payload}.");

    assert!(resolve(&proxy, &unsigned).await.is_err());
}

#[tokio::test]
async fn a_claim_validation_rule_rejects_a_token_that_fails_it() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let mut auth = authenticator(&server.uri(), "proxyauthk8s");
    auth.claim_validation_rules = vec![ClaimValidationRule {
        claim: Some("hd".to_string()),
        required_value: "example.com".to_string(),
        expression: None,
        message: String::new(),
    }];
    let proxy = jwt_proxy(&ns, &cluster, vec![auth]);

    let accepted = sign_claims(&json!({
        "iss": server.uri(), "sub": "1234", "aud": "proxyauthk8s",
        "hd": "example.com", "iat": now(), "exp": now() + 300,
    }));
    assert!(resolve(&proxy, &accepted).await.is_ok());

    let refused = sign_claims(&json!({
        "iss": server.uri(), "sub": "1234", "aud": "proxyauthk8s",
        "hd": "attacker.com", "iat": now(), "exp": now() + 300,
    }));
    let err = resolve(&proxy, &refused).await.unwrap_err();
    assert!(
        matches!(
            err,
            UserAuthError::JwtValidation(
                jwt_validator::JwtValidationError::ClaimRuleRejected { .. }
            )
        ),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn a_cel_claim_rule_and_expression_mappings_are_applied() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let mut auth = authenticator(&server.uri(), "proxyauthk8s");
    auth.claim_validation_rules = vec![ClaimValidationRule {
        claim: None,
        required_value: String::new(),
        expression: Some("claims.email.endsWith('@example.com')".to_string()),
        message: "only example.com identities may use this cluster".to_string(),
    }];
    auth.claim_mappings = ClaimMappings {
        username: Some(PrefixedClaimOrExpression {
            prefix: None,
            claim: None,
            expression: Some("claims.email".to_string()),
        }),
        groups: Some(PrefixedClaimOrExpression {
            prefix: None,
            claim: None,
            expression: Some("claims.groups".to_string()),
        }),
        uid: None,
        extra: vec![ExtraMapping {
            key: "email".to_string(),
            value_expression: "claims.email".to_string(),
        }],
    };
    let proxy = jwt_proxy(&ns, &cluster, vec![auth]);

    let token = sign_claims(&json!({
        "iss": server.uri(), "sub": "1234", "aud": "proxyauthk8s",
        "email": "alice@example.com", "groups": ["dev"],
        "iat": now(), "exp": now() + 300,
    }));

    let user = resolve(&proxy, &token).await.unwrap().unwrap();
    assert_eq!(user.username, "alice@example.com");
    assert_eq!(user.groups, vec!["dev"]);
    assert_eq!(user.email, "alice@example.com");

    let outsider = sign_claims(&json!({
        "iss": server.uri(), "sub": "1234", "aud": "proxyauthk8s",
        "email": "mallory@attacker.com", "groups": ["dev"],
        "iat": now(), "exp": now() + 300,
    }));
    let err = resolve(&proxy, &outsider).await.unwrap_err();
    assert!(
        err.to_string().contains("only example.com identities"),
        "{err}"
    );
}

/// The rule that stops an IdP from minting a privileged identity. It runs on the
/// *mapped* user, which is the only point where the final username is known.
#[tokio::test]
async fn a_user_validation_rule_refuses_a_reserved_username() {
    let server = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let mut auth = authenticator(&server.uri(), "proxyauthk8s");
    auth.claim_mappings.username = Some(claim_mapping("sub", Some("")));
    auth.user_validation_rules = vec![UserValidationRule {
        expression: "!user.username.startsWith('system:')".to_string(),
        message: "username cannot use the reserved system: prefix".to_string(),
    }];
    let proxy = jwt_proxy(&ns, &cluster, vec![auth]);

    let ok = sign_claims(&json!({
        "iss": server.uri(), "sub": "alice", "aud": "proxyauthk8s",
        "iat": now(), "exp": now() + 300,
    }));
    assert!(resolve(&proxy, &ok).await.is_ok());

    let escalation = sign_claims(&json!({
        "iss": server.uri(), "sub": "system:masters", "aud": "proxyauthk8s",
        "iat": now(), "exp": now() + 300,
    }));
    let err = resolve(&proxy, &escalation).await.unwrap_err();
    assert!(
        matches!(
            err,
            UserAuthError::JwtValidation(
                jwt_validator::JwtValidationError::UserRuleRejected { .. }
            )
        ),
        "unexpected error: {err}"
    );
    assert!(err.to_string().contains("reserved system: prefix"), "{err}");
}

/// Several authenticators, each trusted only for the issuer it names.
#[tokio::test]
async fn the_authenticator_is_selected_by_the_token_issuer() {
    let first = issuer_server().await;
    let second = issuer_server().await;
    let (ns, cluster) = unique_cluster();
    let mut second_auth = authenticator(&second.uri(), "second-audience");
    second_auth.claim_mappings.username = Some(claim_mapping("sub", Some("second:")));
    let proxy = jwt_proxy(
        &ns,
        &cluster,
        vec![authenticator(&first.uri(), "first-audience"), second_auth],
    );

    let token = sign_claims(&json!({
        "iss": second.uri(), "sub": "1234", "aud": "second-audience",
        "iat": now(), "exp": now() + 300,
    }));

    let user = resolve(&proxy, &token).await.unwrap().unwrap();
    assert_eq!(user.username, "second:1234");

    // The second issuer's audience must not be accepted for the first issuer.
    let crossed = sign_claims(&json!({
        "iss": first.uri(), "sub": "1234", "aud": "second-audience",
        "iat": now(), "exp": now() + 300,
    }));
    assert!(resolve(&proxy, &crossed).await.is_err());
}

/// Admission forbids this shape, so reaching it means a resource predates the
/// rule — it must fail closed, not authenticate against no issuer at all.
#[tokio::test]
async fn no_configured_authenticator_fails_closed() {
    let (ns, cluster) = unique_cluster();
    let proxy = jwt_proxy(&ns, &cluster, Vec::new());

    let err = resolve(&proxy, "irrelevant").await.unwrap_err();
    assert!(
        matches!(err, UserAuthError::NoJwtAuthenticator),
        "unexpected error: {err}"
    );
}
