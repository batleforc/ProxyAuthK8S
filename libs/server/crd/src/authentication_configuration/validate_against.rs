use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Validate the authentication token against either:
/// - the OIDC provider, by calling the provider's userinfo endpoint and
///   validating the response according to the configured rules;
/// - the Kubernetes API, by calling the `SelfSubjectAccessReview` API;
/// - the configured `jwt` authenticators, locally — verifying the signature
///   against the issuer's JWKS and applying the claim validation rules, claim
///   mappings and user validation rules, as the apiserver's structured
///   authentication configuration does.
///
/// `JwtAuthenticators` is the only mode that needs no round trip to the provider
/// or the target cluster on the request path, so it is also the cheapest — at
/// the cost of not seeing a token revoked before it expires, which `OidcProvider`
/// (via `/userinfo`) does.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
pub enum ValidateAgainst {
    OidcProvider,
    Kubernetes,
    JwtAuthenticators,
}
