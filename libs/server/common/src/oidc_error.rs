use openidconnect::{url::ParseError, HttpClientError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error("Invalid issuer URL: {0}")]
    InvalidIssuerUrl(#[source] ParseError),

    #[error("OIDC discovery error: {0}")]
    OidcDiscovery(#[source] openidconnect::DiscoveryError<HttpClientError<reqwest::Error>>),

    #[error("Token audience validation failed: {0}")]
    AudienceValidation(String),

    #[error("failed to build the HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}

impl From<ParseError> for OidcError {
    fn from(e: ParseError) -> Self {
        OidcError::InvalidIssuerUrl(e)
    }
}

impl From<openidconnect::DiscoveryError<HttpClientError<reqwest::Error>>> for OidcError {
    fn from(e: openidconnect::DiscoveryError<HttpClientError<reqwest::Error>>) -> Self {
        OidcError::OidcDiscovery(e)
    }
}
