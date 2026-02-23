use actix_web::{
    error::{ErrorInternalServerError, ErrorUnauthorized},
    web, FromRequest,
};
use common::{oidc_conf::OidcConf, State};
use crd::ProxyKubeApi;
use k8s_openapi::api::authentication::v1::SelfSubjectReview;
use kube::Api;
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

            match User::get_user_info_from_oidc_token(
                token.to_string(),
                oidc_handler.oidc_client.clone(),
            )
            .await
            {
                Ok(Some(user)) => Ok(user),
                Ok(None) => {
                    tracing::warn!("User info not found in OIDC response");
                    Err(ErrorUnauthorized("User info not found in OIDC response"))
                }
                Err(e) => {
                    tracing::warn!("Error while getting user info from OIDC token: {}", e);
                    Err(ErrorUnauthorized("Invalid user info from OIDC token"))
                }
            }
        })
    }
}

impl User {
    pub fn is_in_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }

    pub async fn get_user_info(
        state: State,
        ns: String,
        cluster: String,
        token: String,
    ) -> Result<Option<Self>, String> {
        let proxy: ProxyKubeApi = match state
            .get_object_from_redis("proxyk8sauth".to_string(), format!("{}/{}", ns, cluster))
            .await
        {
            Ok(Some(proxy)) => proxy,
            Ok(None) => return Err("Proxy not found".to_string()),
            Err(e) => return Err(format!("Error fetching proxy from Redis: {}", e)),
        };
        if proxy.spec.auth_config.clone().is_none() {
            return Ok(None);
        }

        match proxy.spec.auth_config.clone().unwrap().validate_against {
            crd::authentication_configuration::ValidateAgainst::OidcProvider => {
                Self::auth_against_oidc_provider(state, proxy, token).await
            }
            crd::authentication_configuration::ValidateAgainst::Kubernetes => {
                Self::auth_against_kubernetes(state, proxy, token).await
            }
        }
    }

    pub async fn auth_against_kubernetes(
        state: State,
        proxy: ProxyKubeApi,
        token: String,
    ) -> Result<Option<Self>, String> {
        // Create a Kubernetes client using the provided token and targeting the proxy from the request
        let client = proxy
            .to_kube_client(
                state.clone().into(),
                Some("default".to_owned()),
                Some(token),
            )
            .await
            .map_err(|e| {
                tracing::error!("Error while creating Kubernetes client: {}", e);
                format!("Error while creating Kubernetes client: {}", e)
            })?;

        let review: Api<SelfSubjectReview> = Api::all_with(client, &());
        match review.get("").await {
            Ok(review_content) => {
                let user_info = review_content
                    .status
                    .unwrap_or_default()
                    .user_info
                    .unwrap_or_default();
                // In a real implementation, extract user info from the SelfSubjectReview response
                // Here we return a dummy user for illustration
                Ok(Some(User {
                    username: user_info.username.unwrap_or_default(),
                    email: user_info
                        .extra
                        .and_then(|extra| {
                            extra
                                .get("email")
                                .and_then(|emails| emails.first().cloned())
                        })
                        .unwrap_or_default(),
                    groups: user_info.groups.unwrap_or_default(),
                }))
            }
            Err(e) => {
                tracing::warn!(
                    "Error while executing SelfSubjectAccessReview request: {}",
                    e
                );
                Err("Invalid SelfSubjectAccessReview response".to_string())
            }
        }
    }

    pub async fn auth_against_oidc_provider(
        state: State,
        proxy: ProxyKubeApi,
        token: String,
    ) -> Result<Option<Self>, String> {
        let oidc_conf = match proxy.get_oidc_conf(state.clone().into(), false, None) {
            Some(conf) => conf,
            None => {
                tracing::warn!(
                    "No OIDC configuration found for proxy {:?}",
                    proxy.metadata.name
                );
                return Err("No OIDC configuration found for this proxy".to_string());
            }
        };
        Self::get_user_info_from_oidc_token(token, oidc_conf).await
    }

    pub async fn get_user_info_from_oidc_token(
        token: String,
        oidc_conf: OidcConf,
    ) -> Result<Option<Self>, String> {
        let oidc_core = oidc_conf.get_oidc_core().await.map_err(|e| {
            tracing::error!("Error while getting OIDC core client: {}", e);
            format!("Error while getting OIDC core client: {}", e)
        })?;
        let http_client = oidc_conf.get_reqwest_client();

        let user_claim_req = match oidc_core.user_info(AccessToken::new(token.to_owned()), None) {
            Ok(req) => req,
            Err(e) => {
                tracing::warn!("Error while getting user info: {}", e);
                return Err("Invalid user info request".to_string());
            }
        };
        let user_info: GroupsUserInfoClaims = match user_claim_req.request_async(&http_client).await
        {
            Ok(info) => info,
            Err(UserInfoError::Other(err)) => {
                tracing::warn!("Error while executing user info request: {}", err);
                return Err("Invalid user info response".to_string());
            }
            Err(e) => {
                tracing::warn!("Error while executing user info request: {}", e);
                return Err("Invalid user info response".to_string());
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
        Ok(Some(User {
            username,
            email,
            groups,
        }))
    }
}
