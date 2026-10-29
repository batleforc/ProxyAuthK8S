use std::sync::{Arc, LazyLock};

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

use common::{
    State,
    oidc_cache::{
        LEGACY_MAPPING, TtlCache, caching_enabled, mapping_fingerprint, token_cache_key,
        token_ttl_for,
    },
    oidc_conf::OidcConf,
};
use crd::ProxyKubeApi;
use crd_runtime::ProxyKubeApiRuntime;
use k8s_openapi::api::authentication::v1::SelfSubjectReview;
use kube::{Api, api::PostParams};
use openidconnect::{
    AccessToken, AdditionalClaims, UserInfoClaims, UserInfoError, core::CoreGenderClaim,
};
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
    /// The `SelfSubjectReview` through the agent gateway failed (RFC 0012):
    /// the apiserver's status when it answered, and what went wrong.
    #[error("SelfSubjectReview through the agent failed: {message}")]
    AgentSelfSubjectReview {
        status: Option<u16>,
        message: String,
    },
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
            Self::AgentSelfSubjectReview {
                status: Some(401 | 403),
                ..
            } => true,
            // Everything else is configuration or infrastructure: a missing
            // proxy, Redis, building the upstream client, resolving the OIDC
            // config, or an authenticator list that should not be empty.
            Self::ProxyNotFound
            | Self::Redis(_)
            | Self::Runtime(_)
            | Self::SelfSubjectReview(_)
            | Self::AgentSelfSubjectReview { .. }
            | Self::OidcConfigMissing
            | Self::OidcCore(_)
            | Self::OidcHttpClient(_)
            | Self::UserInfoRequest(_)
            | Self::UserInfoUnreachable(_)
            | Self::NoJwtAuthenticator => false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct User {
    pub username: String,
    pub email: String,
    pub groups: Vec<String>,
    /// From `claim_mappings.extra` (RFC 0014), for the apiserver only: the
    /// proxy authorizes on name and groups.
    #[serde(default)]
    pub extra: std::collections::BTreeMap<String, Vec<String>>,
}

/// A cluster's `claim_mappings` and whether its mode is trusted (RFC 0014):
/// how `/userinfo` claims become the user, and how strictly.
#[derive(Clone, Copy)]
pub struct Mapping<'a> {
    pub mappings: &'a crd::authentication_configuration::ClaimMappings,
    pub trusted: bool,
}

/// Every claim `/userinfo` returns, for `claim_mappings` to read.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RawClaims {
    #[serde(flatten)]
    claims: serde_json::Map<String, serde_json::Value>,
}
impl AdditionalClaims for RawClaims {}

/// The caller, resolved from its bearer token against the service-wide OIDC
/// client. Every rejection is a `401`; its reason stays in the logs.
impl FromRequestParts<Arc<State>> for User {
    type Rejection = Response;

    #[instrument(skip_all)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<State>,
    ) -> Result<Self, Self::Rejection> {
        tracing::info!("Start auth middleware");
        let token = match extract_authorization_header(&parts.headers) {
            Ok(token) => token,
            Err(e) => {
                tracing::warn!("Authorization header extraction failed: {}", e);
                return Err(e.into_response());
            }
        };
        match User::get_user_info_from_oidc_token(token.to_string(), state.oidc_client.clone())
            .await
        {
            Ok(Some(user)) => Ok(user),
            Ok(None) => {
                tracing::warn!("User info not found in OIDC response");
                Err((
                    StatusCode::UNAUTHORIZED,
                    "User info not found in OIDC response",
                )
                    .into_response())
            }
            Err(e) => {
                tracing::warn!("Error while getting user info from OIDC token: {}", e);
                Err((
                    StatusCode::UNAUTHORIZED,
                    "Invalid user info from OIDC token",
                )
                    .into_response())
            }
        }
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
        let proxy: ProxyKubeApi = match state.get_proxy_from_redis(&format!("{ns}/{cluster}")).await
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
                let trusted = proxy.upstream_mode().is_trusted();
                Self::auth_against_jwt_authenticators_for(state, &auth_config.jwt, &token, trusted)
                    .await
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
        Self::auth_against_jwt_authenticators_for(state, authenticators, token, false).await
    }

    /// [`Self::auth_against_jwt_authenticators`], for a cluster whose mode is
    /// `trusted` (RFC 0014, decision 51).
    pub async fn auth_against_jwt_authenticators_for(
        state: State,
        authenticators: &[crd::authentication_configuration::JWTAuthenticator],
        token: &str,
        trusted: bool,
    ) -> Result<Option<Self>, UserAuthError> {
        if authenticators.is_empty() {
            return Err(UserAuthError::NoJwtAuthenticator);
        }
        let mapped = state
            .jwt_validator
            .validate_for(token, authenticators, trusted)
            .await?;
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
            extra: mapped.extra,
        }))
    }

    #[instrument(skip(state, proxy, token))]
    pub async fn auth_against_kubernetes(
        state: State,
        proxy: ProxyKubeApi,
        token: String,
    ) -> Result<Option<Self>, UserAuthError> {
        let reviewed = if matches!(proxy.spec.service, crd::service::Service::Agent(_)) {
            // RFC 0012: no kube client reaches an agent cluster; the review
            // goes through the agent gateway like any relayed request.
            crate::cluster::redirect::kube_redirect::self_subject_review_through_agent(
                &proxy, &state, &token,
            )
            .await
            .map_err(|(status, message)| {
                tracing::warn!(
                    "Error while executing SelfSubjectReview request: {}",
                    message
                );
                UserAuthError::AgentSelfSubjectReview { status, message }
            })?
        } else {
            // A Kubernetes client with the caller's token, to the proxy's target.
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
            review
                .create(&PostParams::default(), &SelfSubjectReview::default())
                .await
                .map_err(|e| {
                    tracing::warn!("Error while executing SelfSubjectReview request: {}", e);
                    UserAuthError::SelfSubjectReview(e)
                })?
        };
        let user_info = reviewed
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
            ..Self::default()
        }))
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
        let mappings = proxy
            .spec
            .auth_config
            .as_ref()
            .and_then(|config| config.oidc_provider.claim_mappings.as_ref());
        let mapping = mappings.map(|mappings| Mapping {
            mappings,
            trusted: proxy.upstream_mode().is_trusted(),
        });
        Self::get_user_info_mapped(token, oidc_conf, mapping).await
    }

    pub async fn get_user_info_from_oidc_token(
        token: String,
        oidc_conf: OidcConf,
    ) -> Result<Option<Self>, UserAuthError> {
        Self::get_user_info_mapped(token, oidc_conf, None).await
    }

    /// The user of `token` per `/userinfo`, through `mapping` when the cluster
    /// has `claim_mappings`, else `preferred_username` and `groups`.
    #[instrument(skip(token, oidc_conf, mapping))]
    pub async fn get_user_info_mapped(
        token: String,
        oidc_conf: OidcConf,
        mapping: Option<Mapping<'_>>,
    ) -> Result<Option<Self>, UserAuthError> {
        let fingerprint = mapping.map_or_else(
            || LEGACY_MAPPING.to_string(),
            |mapping| mapping_fingerprint(mapping.mappings, mapping.trusted),
        );
        let cache_key = token_cache_key(
            &token,
            &oidc_conf.issuer_url,
            &oidc_conf.client_id,
            &oidc_conf.audience,
            &fingerprint,
        );
        if let Some(user) = VALIDATED_TOKEN_CACHE.get(&cache_key) {
            tracing::debug!("OIDC token served from the validated-token cache");
            return Ok(Some(user));
        }
        let user = Self::validate_oidc_token(&token, &oidc_conf, mapping).await?;
        if caching_enabled() {
            VALIDATED_TOKEN_CACHE.insert(cache_key, user.clone(), token_ttl_for(&token));
        }
        Ok(Some(user))
    }

    /// Validate `token` against the provider (`/userinfo`, then audience) and
    /// resolve the user. Always hits the IdP; see [`Self::get_user_info_from_oidc_token`].
    async fn validate_oidc_token(
        token: &str,
        oidc_conf: &OidcConf,
        mapping: Option<Mapping<'_>>,
    ) -> Result<Self, UserAuthError> {
        if let Some(mapping) = mapping {
            let user_info: UserInfoClaims<RawClaims, CoreGenderClaim> =
                Self::fetch_userinfo(token, oidc_conf).await?;
            if let Err(e) = oidc_conf.ensure_token_audience(token).await {
                tracing::warn!("Token rejected by audience validation: {}", e);
                return Err(UserAuthError::AudienceValidation(e.to_string()));
            }
            let claims = serde_json::to_value(&user_info).unwrap_or_default();
            let mapped = jwt_validator::map_user_for(mapping.mappings, &claims, mapping.trusted)?;
            tracing::debug!(group_count = mapped.groups.len(), "mapped /userinfo claims");
            return Ok(User {
                email: claims
                    .get("email")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                username: mapped.username,
                groups: mapped.groups,
                extra: mapped.extra,
            });
        }
        let user_info: GroupsUserInfoClaims = Self::fetch_userinfo(token, oidc_conf).await?;
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
            ..User::default()
        })
    }

    /// `/userinfo` for `token`, as `AC` claims. A transport failure, a 5xx or
    /// a body above `PROXY_JWKS_MAX_DOCUMENT_BYTES` (RFC 0014 §7.3) is
    /// `UserInfoUnreachable`: the provider's doing, never the caller's fault.
    async fn fetch_userinfo<AC: AdditionalClaims>(
        token: &str,
        oidc_conf: &OidcConf,
    ) -> Result<UserInfoClaims<AC, CoreGenderClaim>, UserAuthError> {
        let oidc_core = oidc_conf.oidc_core().await.map_err(|e| {
            tracing::error!("Error while getting OIDC core client: {}", e);
            UserAuthError::OidcCore(e.to_string())
        })?;
        let client = oidc_conf.reqwest_client().map_err(|e| {
            tracing::error!("Error while building OIDC http client: {}", e);
            UserAuthError::OidcHttpClient(e.to_string())
        })?;
        let max = common::config::get().proxy.jwks_max_document_bytes;
        let http_client = move |request: openidconnect::HttpRequest| {
            let client = client.clone();
            async move { capped_call(&client, request, max).await }
        };

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
        match user_claim_req.request_async(&http_client).await {
            Ok(info) => Ok(info),
            // Transport failure (DNS, connect, TLS, timeout) or the provider
            // answering 5xx: nothing about the caller's token would have changed
            // the outcome, so it must not count toward their fail2login budget.
            Err(UserInfoError::Request(err)) => {
                tracing::warn!("Could not reach the OIDC userinfo endpoint: {}", err);
                Err(UserAuthError::UserInfoUnreachable(err.to_string()))
            }
            Err(UserInfoError::Response(status, _, message)) if status.is_server_error() => {
                tracing::warn!(%status, "OIDC userinfo endpoint returned a server error");
                Err(UserAuthError::UserInfoUnreachable(message))
            }
            Err(UserInfoError::Other(err)) => {
                tracing::warn!("Error while executing user info request: {}", err);
                Err(UserAuthError::UserInfoResponse)
            }
            Err(e) => {
                tracing::warn!("Error while executing user info request: {:#?}", e);
                Err(UserAuthError::UserInfoResponse)
            }
        }
    }
}

/// Why a bounded `/userinfo` call failed.
#[derive(Debug, thiserror::Error)]
enum CappedCallError {
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("response larger than {0} bytes")]
    TooLarge(usize),
    #[error("{0}")]
    Response(#[from] http::Error),
}

/// `request` through `client`, the response body read in chunks and refused
/// past `max` bytes, `Content-Length` or not.
async fn capped_call(
    client: &reqwest::Client,
    request: openidconnect::HttpRequest,
    max: usize,
) -> Result<openidconnect::HttpResponse, CappedCallError> {
    let mut response = client.execute(reqwest::Request::try_from(request)?).await?;
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(CappedCallError::TooLarge(max));
    }
    let mut builder = http::Response::builder().status(response.status());
    for (name, value) in response.headers() {
        builder = builder.header(name, value);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > max {
            return Err(CappedCallError::TooLarge(max));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(builder.body(body)?)
}
