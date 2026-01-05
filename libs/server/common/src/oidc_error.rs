use openidconnect::{url::ParseError, HttpClientError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error("Invalid issuer URL: {0}")]
    InvalidIssuerUrl(#[source] ParseError),

    #[error("OIDC discovery error: {0}")]
    OidcDiscovery(#[source] openidconnect::DiscoveryError<HttpClientError<reqwest::Error>>),
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
