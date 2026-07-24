use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Validate the authentication token against either:
/// - the OIDC provider, by validating the token by calling the provider's userinfo endpoint and validating the response according to the configured rules
/// - the kubernetes API, by validating the token by calling the SelfSubjectAccessReview API
///
/// Validating JWTs locally against the `jwt` authenticators — signature, claim
/// validation rules and claim mappings, as the apiserver's structured
/// authentication configuration does — is on the roadmap and has no variant
/// here yet: offering the option before it is enforced would let a cluster
/// believe it validates tokens it does not.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum ValidateAgainst {
    OidcProvider,
    Kubernetes,
}
