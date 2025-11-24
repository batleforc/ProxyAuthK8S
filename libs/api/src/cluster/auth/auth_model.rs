use openidconnect::{Nonce, PkceCodeVerifier};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, Serialize, ToSchema)]
pub struct LoginToCallBackModel {
    pub nonce: String,
    pub pkce_verifier: String,
}

impl LoginToCallBackModel {
    pub fn new(nonce: String, pkce_verifier: String) -> Self {
        LoginToCallBackModel {
            nonce,
            pkce_verifier,
        }
    }
    pub fn from_string(s: &str) -> Option<LoginToCallBackModel> {
        match serde_json::from_str::<LoginToCallBackModel>(s) {
            Ok(model) => Some(model),
            Err(_) => None,
        }
    }
    pub fn to_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
    pub fn get_nonce(&self) -> Nonce {
        Nonce::new(self.nonce.clone())
    }
    pub fn get_pkce_verifier(&self) -> PkceCodeVerifier {
        PkceCodeVerifier::new(self.pkce_verifier.clone())
    }
}
