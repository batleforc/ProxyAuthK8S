use actix_web::{
    FromRequest,
    error::{ErrorInternalServerError, ErrorUnauthorized},
    web,
};
use std::sync::LazyLock;

use common::{
    State,
    oidc_cache::{TtlCache, caching_enabled, token_cache_key, token_ttl_for},
    oidc_conf::OidcConf,
};
use crd::ProxyKubeApi;
use crd_runtime::ProxyKubeApiRuntime;
use k8s_openapi::api::authentication::v1::SelfSubjectReview;
use kube::{Api, api::PostParams};
use openidconnect::{AccessToken, UserInfoError};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{helper::extract_authorization_header, model::user_claim::GroupsUserInfoClaims};

/// Users resolved from a validated OIDC token (see [`common::oidc_cache`]):
/// keyed by a hash of the token + issuer/client/audience, short TTL, successes only.
static VALIDATED_TOKEN_CACHE: LazyLock<TtlCache<String, User>> = LazyLock::new(TtlCache::default);

/// Failure resolving a caller's identity for a proxied cluster.
///
/// Distinguishes the underlying cause (Redis, missing proxy, upstream client
/// build, Kubernetes `SelfSubjectReview`, OIDC discovery/userinfo/audience) for
/// logging and the controller status. Callers on the request path still collapse
/// every variant into an opaque 401 — the detail never reaches the client.
#[derive(Debug, thiserror::Error)]
pub enum UserAuthError {
    /// No `ProxyKubeApi` matched the requested namespace/cluster.
    #[error("proxy not found")]
    ProxyNotFound,
    /// The proxy lookup against Redis failed.
    #[error("error fetching proxy from redis: {0}")]
    Redis(String),
    /// Building the upstream kube client for the caller's token failed.
    #[error(transparent)]
    Runtime(#[from] crd_runtime::ProxyRuntimeError),
    /// The `SelfSubjectReview` request to the target cluster failed.
    #[error("SelfSubjectReview request failed: {0}")]
    SelfSubjectReview(#[source] kube::Error),
    /// The proxy has no usable OIDC configuration.
    #[error("no OIDC configuration found for this proxy")]
    OidcConfigMissing,
    /// Discovering / building the OIDC core client failed.
    #[error("error getting OIDC core client: {0}")]
    OidcCore(String),
    /// Building the OIDC HTTP client failed.
    #[error("error building OIDC http client: {0}")]
    OidcHttpClient(String),
    /// The `/userinfo` request could not be constructed.
    #[error("invalid user info request: {0}")]
    UserInfoRequest(String),
    /// The `/userinfo` response was missing or invalid.
    #[error("invalid user info response")]
    UserInfoResponse,
    /// The provider's `/userinfo` endpoint could not be reached, or answered
    /// with a server error.
    ///
    /// Distinct from [`Self::UserInfoResponse`] because it is *not* the caller's
    /// fault: collapsing the two would let a provider outage ban every client
    /// that happened to be calling during it.
    #[error("could not reach the OIDC provider's userinfo endpoint: {0}")]
    UserInfoUnreachable(String),
    /// The token failed audience validation.
    #[error("token audience validation failed: {0}")]
    AudienceValidation(String),
    /// Local validation against the configured `jwt` authenticators failed.
    #[error(transparent)]
    JwtValidation(#[from] jwt_validator::JwtValidationError),
    /// `validate_against: JwtAuthenticators` with an empty `jwt` list.
    ///
    /// Admission rejects this shape, so reaching it means the resource predates
    /// the rule — fail closed rather than authenticate against no issuer at all.
    #[error("no jwt authenticator is configured for this proxy")]
    NoJwtAuthenticator,
}

impl UserAuthError {
    /// Whether this failure is attributable to the *caller's token* rather than
    /// to this server, Redis, the target cluster or the OIDC provider.
    ///
    /// Drives the fail2login counter. Without the distinction, a transient
    /// server-side fault — an unreadable `config_from` Secret, a Redis blip, an
    /// apiserver hiccup — is charged to whichever client happened to be calling,
    /// and enough of them ban a caller for something they did not do. Only the
    /// variants a different, valid token would have avoided count.
    #[must_use]
    pub fn is_caller_fault(&self) -> bool {
        match self {
            // The provider or the audience check refused this token.
            Self::UserInfoResponse | Self::AudienceValidation(_) => true,
            // Delegated: the JWT validator draws the same line internally.
            Self::JwtValidation(e) => e.is_caller_fault(),
            // The target cluster answered, and what it said is about the token.
            // Any other kube error (connection refused, TLS failure, 5xx) is our
            // side of the wire.
            Self::SelfSubjectReview(kube::Error::Api(status)) => {
                status.code == 401 || status.code == 403
            }
            // Everything else is configuration or infrastructure: a missing
            // proxy, Redis, building the upstream client, resolving the OIDC
            // config, or an authenticator list that should not be empty.
            Self::ProxyNotFound
            | Self::Redis(_)
            | Self::Runtime(_)
            | Self::SelfSubjectReview(_)
            | Self::OidcConfigMissing
            | Self::OidcCore(_)
            | Self::OidcHttpClient(_)
            | Self::UserInfoRequest(_)
            | Self::UserInfoUnreachable(_)
            | Self::NoJwtAuthenticator => false,
        }
    }
}

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
            let oidc_handler = if let Some(handler) = req.app_data::<web::Data<State>>() {
                handler.clone()
            } else {
                tracing::error!("Error while getting oidc handler");
                return Err(ErrorInternalServerError("Invalid OIDC handler"));
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
    #[must_use]
    pub fn is_in_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }

    pub async fn get_user_info(
        state: State,
        ns: String,
        cluster: String,
        token: String,
    ) -> Result<Option<Self>, UserAuthError> {
        let proxy: ProxyKubeApi = match state
            .get_object_from_redis(crd::REDIS_PREFIX, &format!("{ns}/{cluster}"))
            .await
        {
            Ok(Some(proxy)) => proxy,
            Ok(None) => return Err(UserAuthError::ProxyNotFound),
            Err(e) => return Err(UserAuthError::Redis(e.to_string())),
        };
        User::get_user_info_with_proxy(state, proxy, token).await
    }

    #[instrument(skip(state, proxy, token))]
    pub async fn get_user_info_with_proxy(
        state: State,
        proxy: ProxyKubeApi,
        token: String,
    ) -> Result<Option<Self>, UserAuthError> {
        let Some(auth_config) = proxy.spec.auth_config.clone() else {
            return Ok(None);
        };

        match auth_config.validate_against {
            crd::authentication_configuration::ValidateAgainst::OidcProvider => {
                Self::auth_against_oidc_provider(state, proxy, token).await
            }
            crd::authentication_configuration::ValidateAgainst::Kubernetes => {
                Self::auth_against_kubernetes(state, proxy, token).await
            }
            crd::authentication_configuration::ValidateAgainst::JwtAuthenticators => {
                Self::auth_against_jwt_authenticators(state, &auth_config.jwt, &token).await
            }
        }
    }

    /// Validate the token locally against the configured `jwt` authenticators.
    ///
    /// Unlike the other two modes this makes no call to the OIDC provider or the
    /// target cluster on the request path — only, at most, a cached JWKS fetch.
    /// The trade-off is that a token revoked before it expires is still accepted
    /// until it does, which `/userinfo` would have caught.
    #[instrument(skip(state, authenticators, token))]
    pub async fn auth_against_jwt_authenticators(
        state: State,
        authenticators: &[crd::authentication_configuration::JWTAuthenticator],
        token: &str,
    ) -> Result<Option<Self>, UserAuthError> {
        if authenticators.is_empty() {
            return Err(UserAuthError::NoJwtAuthenticator);
        }
        let mapped = state.jwt_validator.validate(token, authenticators).await?;
        tracing::debug!(
            username = %mapped.username,
            group_count = mapped.groups.len(),
            "token validated locally against a jwt authenticator"
        );
        Ok(Some(User {
            // The apiserver's claim mappings have no notion of an email; the
            // proxy only uses it for display, so it is read from the mapped
            // `extra` when an operator chose to map one there.
            email: mapped
                .extra
                .get("email")
                .and_then(|values| values.first().cloned())
                .unwrap_or_default(),
            username: mapped.username,
            groups: mapped.groups,
        }))
    }

    #[instrument(skip(state, proxy, token))]
    pub async fn auth_against_kubernetes(
        state: State,
        proxy: ProxyKubeApi,
        token: String,
    ) -> Result<Option<Self>, UserAuthError> {
        // Create a Kubernetes client using the provided token and targeting the proxy from the request
        let client = proxy
            .to_kube_client(
                state.clone().into(),
                Some("default".to_owned()),
                Some(token),
            )
            .await
            .inspect_err(|e| {
                tracing::error!("Error while creating Kubernetes client: {}", e);
            })?;
        let review: Api<SelfSubjectReview> = Api::all(client);

        match review
            .create(&PostParams::default(), &SelfSubjectReview::default())
            .await
        {
            Ok(review_content) => {
                let user_info = review_content
                    .status
                    .unwrap_or_default()
                    .user_info
                    .unwrap_or_default();
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
                tracing::warn!("Error while executing SelfSubjectReview request: {}", e);
                Err(UserAuthError::SelfSubjectReview(e))
            }
        }
    }

    #[instrument(skip(state, proxy, token))]
    pub async fn auth_against_oidc_provider(
        state: State,
        proxy: ProxyKubeApi,
        token: String,
    ) -> Result<Option<Self>, UserAuthError> {
        // `?` maps a resolution failure — typically an unreadable
        // `config_from` Secret — to `UserAuthError::Runtime`, the same variant a
        // failed certificate read already takes, so it stays distinct from
        // `OidcConfigMissing` in the logs while the client still sees the same
        // opaque 401.
        let oidc_conf = match proxy
            .get_oidc_conf(state.clone().into(), false, None)
            .await
            .inspect_err(|e| {
                tracing::warn!(
                    error = %e,
                    proxy = ?proxy.metadata.name,
                    "Could not resolve the OIDC configuration for proxy"
                );
            })? {
            Some(conf) => conf,
            None => {
                tracing::warn!(
                    "No OIDC configuration found for proxy {:?}",
                    proxy.metadata.name
                );
                return Err(UserAuthError::OidcConfigMissing);
            }
        };
        tracing::debug!(
            issuer_url = %oidc_conf.issuer_url,
            client_id = %oidc_conf.client_id,
            "OIDC configuration found for proxy"
        );
        Self::get_user_info_from_oidc_token(token, oidc_conf).await
    }

    #[instrument(skip(token, oidc_conf))]
    pub async fn get_user_info_from_oidc_token(
        token: String,
        oidc_conf: OidcConf,
    ) -> Result<Option<Self>, UserAuthError> {
        let cache_key = token_cache_key(
            &token,
            &oidc_conf.issuer_url,
            &oidc_conf.client_id,
            &oidc_conf.audience,
        );
        if let Some(user) = VALIDATED_TOKEN_CACHE.get(&cache_key) {
            tracing::debug!("OIDC token served from the validated-token cache");
            return Ok(Some(user));
        }
        let user = Self::validate_oidc_token(&token, &oidc_conf).await?;
        if caching_enabled() {
            VALIDATED_TOKEN_CACHE.insert(cache_key, user.clone(), token_ttl_for(&token));
        }
        Ok(Some(user))
    }

    /// Validate `token` against the provider (`/userinfo`, then audience) and
    /// resolve the user. Always hits the IdP; see [`Self::get_user_info_from_oidc_token`].
    async fn validate_oidc_token(token: &str, oidc_conf: &OidcConf) -> Result<Self, UserAuthError> {
        let oidc_core = oidc_conf.oidc_core().await.map_err(|e| {
            tracing::error!("Error while getting OIDC core client: {}", e);
            UserAuthError::OidcCore(e.to_string())
        })?;
        let http_client = oidc_conf.oidc_reqwest_client().map_err(|e| {
            tracing::error!("Error while building OIDC http client: {}", e);
            UserAuthError::OidcHttpClient(e.to_string())
        })?;

        tracing::debug!(
            "Creating user info request for OIDC provider with token of length: {}",
            token.len()
        );
        let user_claim_req = match oidc_core.user_info(AccessToken::new(token.to_string()), None) {
            Ok(req) => req,
            Err(e) => {
                tracing::warn!("Error while creating user info request: {}", e);
                return Err(UserAuthError::UserInfoRequest(e.to_string()));
            }
        };
        let user_info: GroupsUserInfoClaims = match user_claim_req.request_async(&http_client).await
        {
            Ok(info) => info,
            // Transport failure (DNS, connect, TLS, timeout) or the provider
            // answering 5xx: nothing about the caller's token would have changed
            // the outcome, so it must not count toward their fail2login budget.
            Err(UserInfoError::Request(err)) => {
                tracing::warn!("Could not reach the OIDC userinfo endpoint: {}", err);
                return Err(UserAuthError::UserInfoUnreachable(err.to_string()));
            }
            Err(UserInfoError::Response(status, _, message)) if status.is_server_error() => {
                tracing::warn!(%status, "OIDC userinfo endpoint returned a server error");
                return Err(UserAuthError::UserInfoUnreachable(message));
            }
            Err(UserInfoError::Other(err)) => {
                tracing::warn!("Error while executing user info request: {}", err);
                return Err(UserAuthError::UserInfoResponse);
            }
            Err(e) => {
                tracing::warn!("Error while executing user info request: {:#?}", e);
                return Err(UserAuthError::UserInfoResponse);
            }
        };
        // `/userinfo` proved the token is validly signed and active, but not that
        // it was minted for THIS service. Enforce the audience now, before the
        // token's groups are trusted for authorization. (Must run after userinfo
        // so the JWT-claims fallback can trust the — now verified — signature.)
        if let Err(e) = oidc_conf.ensure_token_audience(token).await {
            tracing::warn!("Token rejected by audience validation: {}", e);
            return Err(UserAuthError::AudienceValidation(e.to_string()));
        }
        let email = if let Some(email) = user_info.email() {
            email.to_string()
        } else {
            tracing::warn!("No email found in user info");
            String::new()
        };
        let username = if let Some(username) = user_info.preferred_username() {
            username.to_string()
        } else {
            tracing::warn!("No preferred username found in user info");
            String::new()
        };
        let groups = user_info.additional_claims().groups.clone();
        tracing::debug!(group_count = groups.len(), "resolved user groups");
        Ok(User {
            username,
            email,
            groups,
        })
    }
}
