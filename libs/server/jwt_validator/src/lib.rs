//! Local JWT validation against the `jwt` authenticators of a `ProxyKubeApi`.
//!
//! Mirrors the apiserver's structured authentication configuration: verify the
//! signature against the issuer's JWKS, then apply the claim validation rules,
//! claim mappings and user validation rules.

pub mod cel_rules;
pub mod error;
pub mod jwks;
pub mod validator;

pub use cel_rules::MappedUser;
pub use error::JwtValidationError;
pub use jwks::JwksCache;
pub use validator::{JwtValidator, SharedJwtValidator};
