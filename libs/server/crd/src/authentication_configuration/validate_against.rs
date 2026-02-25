use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Validate the authentication token against either:
/// - [TODO] the JWT authenticators, by validating the token signature and claims according to the configured rules
/// - the OIDC provider, by validating the token by calling the provider's userinfo endpoint and validating the response according to the configured rules
/// - the kubernetes API, by validating the token by calling the SelfSubjectAccessReview API
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum ValidateAgainst {
    // TODO : JwtAuthenticators,
    OidcProvider,
    Kubernetes,
}
