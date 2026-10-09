//! Error type for local JWT validation.

/// Why a token was refused, or why an authenticator could not be applied.
///
/// Every variant is logged and none reaches the client, which only ever sees an
/// opaque 401 — a validation error that told a caller *which* rule rejected them
/// would be a probing oracle for the cluster's authentication policy.
#[derive(Debug, thiserror::Error)]
pub enum JwtValidationError {
    /// The bearer token is not a well-formed JWT.
    #[error("token is not a well-formed JWT: {0}")]
    Malformed(String),
    /// No configured authenticator names the token's `iss`.
    #[error("no configured jwt authenticator matches issuer {issuer}")]
    UnknownIssuer { issuer: String },
    /// The token's header names an algorithm this validator refuses.
    #[error("unsupported or unsafe token algorithm: {alg}")]
    UnsupportedAlgorithm { alg: String },
    /// The issuer's JWKS could not be fetched.
    #[error("failed to fetch JWKS from {url}: {source}")]
    JwksFetch {
        url: String,
        #[source]
        source: Box<reqwest::Error>,
    },
    /// The JWKS document was not usable.
    #[error("JWKS from {url} is not usable: {reason}")]
    JwksInvalid { url: String, reason: String },
    /// No key in the issuer's JWKS matches the token's `kid`.
    #[error("no key in the issuer's JWKS matches kid {kid:?}")]
    UnknownKey { kid: Option<String> },
    /// The signature, `exp`/`nbf`, `iss` or `aud` check failed.
    #[error("token rejected: {0}")]
    Rejected(String),
    /// A configured CEL expression could not be compiled.
    #[error("CEL expression {expression:?} does not compile: {reason}")]
    ExpressionCompile { expression: String, reason: String },
    /// A configured CEL expression failed at evaluation time.
    #[error("CEL expression {expression:?} failed to evaluate: {reason}")]
    ExpressionEvaluate { expression: String, reason: String },
    /// A CEL expression produced the wrong type for where it is used.
    #[error("CEL expression {expression:?} produced {got}, expected {want}")]
    ExpressionType {
        expression: String,
        got: String,
        want: String,
    },
    /// A `claim_validation_rules` entry rejected the token.
    #[error("claim validation rule rejected the token: {message}")]
    ClaimRuleRejected { message: String },
    /// A `user_validation_rules` entry rejected the mapped user.
    #[error("user validation rule rejected the user: {message}")]
    UserRuleRejected { message: String },
    /// A claim a mapping needs is missing or the wrong type.
    #[error("claim {claim:?} is missing or not usable as {want}")]
    ClaimUnusable { claim: String, want: &'static str },
    /// The authenticator's own configuration is invalid.
    #[error("jwt authenticator configuration is invalid: {0}")]
    Configuration(String),
}

impl JwtValidationError {
    /// Whether this failure is attributable to the *caller's token* rather than
    /// to this server or the issuer being unreachable.
    ///
    /// Drives the fail2login counter: a caller must not accumulate a ban because
    /// the issuer's JWKS endpoint was briefly down or an authenticator is
    /// misconfigured. Only the variants a different, valid token would have
    /// avoided count.
    #[must_use]
    pub fn is_caller_fault(&self) -> bool {
        match self {
            // The token itself is wrong: malformed, from an issuer nobody
            // trusts, signed with a key or algorithm we refuse, or rejected by
            // the configured rules.
            Self::Malformed(_)
            | Self::UnknownIssuer { .. }
            | Self::UnsupportedAlgorithm { .. }
            | Self::UnknownKey { .. }
            | Self::Rejected(_)
            | Self::ClaimRuleRejected { .. }
            | Self::UserRuleRejected { .. }
            | Self::ClaimUnusable { .. } => true,
            // Our side, or the issuer's: unreachable JWKS, an unusable JWKS
            // document, a rule that does not compile, a broken authenticator.
            Self::JwksFetch { .. }
            | Self::JwksInvalid { .. }
            | Self::ExpressionCompile { .. }
            | Self::ExpressionEvaluate { .. }
            | Self::ExpressionType { .. }
            | Self::Configuration(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JwtValidationError;

    /// A caller must not accumulate a ban because the issuer was briefly
    /// unreachable or an authenticator is misconfigured — neither is something
    /// a different, valid token would have avoided.
    #[test]
    fn only_token_faults_count_against_the_caller() {
        assert!(JwtValidationError::Malformed("x".to_string()).is_caller_fault());
        assert!(JwtValidationError::Rejected("bad signature".to_string()).is_caller_fault());
        assert!(
            JwtValidationError::UnknownIssuer {
                issuer: "https://evil.example.com".to_string()
            }
            .is_caller_fault()
        );
        assert!(
            JwtValidationError::ClaimRuleRejected {
                message: "nope".to_string()
            }
            .is_caller_fault()
        );

        assert!(
            !JwtValidationError::JwksInvalid {
                url: "https://issuer.example.com".to_string(),
                reason: "no keys".to_string(),
            }
            .is_caller_fault()
        );
        assert!(!JwtValidationError::Configuration("broken".to_string()).is_caller_fault());
        assert!(
            !JwtValidationError::ExpressionCompile {
                expression: "claims.".to_string(),
                reason: "parse error".to_string(),
            }
            .is_caller_fault()
        );
    }
}
