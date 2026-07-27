use std::fmt::{Display, Formatter};

use openidconnect::{Nonce, PkceCodeVerifier};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, Serialize, ToSchema)]
pub struct LoginToCallBackModel {
    pub nonce: String,
    pub pkce_verifier: String,
}

impl LoginToCallBackModel {
    #[must_use]
    pub fn new(nonce: String, pkce_verifier: String) -> Self {
        LoginToCallBackModel {
            nonce,
            pkce_verifier,
        }
    }
    #[must_use]
    pub fn from_string(s: &str) -> Option<LoginToCallBackModel> {
        serde_json::from_str::<LoginToCallBackModel>(s).ok()
    }
    #[must_use]
    pub fn nonce(&self) -> Nonce {
        Nonce::new(self.nonce.clone())
    }
    #[must_use]
    pub fn pkce_verifier(&self) -> PkceCodeVerifier {
        PkceCodeVerifier::new(self.pkce_verifier.clone())
    }
}

impl Display for LoginToCallBackModel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Two plain `String` fields always serialize; treat a failure as a
        // formatting error rather than panicking inside `Display`.
        match serde_json::to_string(self) {
            Ok(json) => write!(f, "{json}"),
            Err(_) => Err(std::fmt::Error),
        }
    }
}
