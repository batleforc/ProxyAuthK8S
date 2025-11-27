use actix_web::{
    error::{ErrorInternalServerError, ErrorUnauthorized},
    web, FromRequest,
};
use common::State;
use openidconnect::{AccessToken, UserInfoError};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{helper::extract_authorization_header, model::user_claim::GroupsUserInfoClaims};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct User {
    pub username: String,
    pub email: String,
    pub groups: Vec<String>,
}

impl FromRequest for User {
    type Error = actix_web::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, Self::Error>>>>;

    // https://github.com/batleforc/rust-template/blob/main/src/model/user.rs
    #[instrument(skip(_payload, req))]
    fn from_request(
        req: &actix_web::HttpRequest,
        _payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        let req = req.clone();
        tracing::info!("Start auth middleware");
        Box::pin(async move {
            let token = match extract_authorization_header(&req) {
                Ok(token) => token,
                Err(e) => {
                    tracing::warn!("Authorization header extraction failed: {}", e);
                    return Err(e.into_actix_error());
                }
            };
            let oidc_handler = match req.app_data::<web::Data<State>>() {
                Some(handler) => handler.clone(),
                None => {
                    tracing::error!("Error while getting oidc handler");
                    return Err(ErrorInternalServerError("Invalid OIDC handler"));
                }
            };

            let oidc_conf = match oidc_handler.oidc_client.get_oidc_core().await {
                Ok(conf) => conf,
                Err(e) => {
                    tracing::error!("Error while getting OIDC config: {}", e);
                    return Err(ErrorInternalServerError("Invalid OIDC config"));
                }
            };
            let http_client = oidc_handler.oidc_client.get_reqwest_client();

            let user_claim_req = match oidc_conf.user_info(AccessToken::new(token.to_owned()), None)
            {
                Ok(req) => req,
                Err(e) => {
                    tracing::warn!("Error while getting user info: {}", e);
                    return Err(ErrorInternalServerError("Invalid user info request"));
                }
            };
            let user_info: GroupsUserInfoClaims =
                match user_claim_req.request_async(&http_client).await {
                    Ok(info) => info,
                    Err(UserInfoError::Other(err)) => {
                        tracing::warn!("Error while executing user info request: {}", err);
                        return Err(ErrorInternalServerError("Invalid user info response"));
                    }
                    Err(e) => {
                        tracing::warn!("Error while executing user info request: {}", e);
                        return Err(ErrorUnauthorized("Invalid user info response"));
                    }
                };
            let email = match user_info.email() {
                Some(email) => email.to_string(),
                None => {
                    tracing::warn!("No email found in user info");
                    "".to_string()
                }
            };
            let username = match user_info.preferred_username() {
                Some(username) => username.to_string(),
                None => {
                    tracing::warn!("No preferred username found in user info");
                    "".to_string()
                }
            };
            let groups = user_info.additional_claims().groups.clone();
            tracing::info!("User groups: {:?}", groups);
            // In a real implementation, extract user info from request (e.g., headers, tokens)
            // Here we return a dummy user for illustration
            let user = User {
                username,
                email,
                groups,
            };
            Ok(user)
        })
    }
}
