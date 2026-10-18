use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use crd::ProxyKubeApi;
use crd_runtime::ProxyKubeApiRuntime;
use openidconnect::{AccessTokenHash, AuthorizationCode, OAuth2TokenResponse, TokenResponse};
use serde::Deserialize;
use tracing::{error, info, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::{
    auth_model::LoginToCallBackModel, callback_model::CallbackModel, throttle_oauth_as,
};
use crate::helper::NsCluster;

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct CallbackQuery {
    /// Authorization code from the OIDC provider.
    pub code: String,
    /// State parameter to prevent CSRF.
    pub state: String,
}

/// Callback from the cluster's OIDC provider
///
/// If the cluster is not found or disabled, return 404.
#[utoipa::path(
    get,
    path = "/{ns}/{cluster}/auth/callback",
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster.", body = CallbackModel),
        (status = 400, description = "Missing or already-used CSRF state."),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 429, description = "Rate limited or banned for this cluster."),
        (status = 500, description = "Internal server error."),
        (status = 503, description = "Redis is unavailable."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
        ("x-front-callback" = String, Header, nullable, description = "If it's from the frontend, this header will be set."),
        ("x-kubectl-callback" = String, Header, nullable, description = "If it's from kubectl plugin, this header will be set."),
        CallbackQuery,
    )
)]
#[instrument(name = "cluster_callback", skip(data, req, callback))]
pub async fn callback_login(
    State(data): State<Arc<common::State>>,
    Path(NsCluster { ns, cluster }): Path<NsCluster>,
    req: Parts,
    callback: Query<CallbackQuery>,
) -> Response {
    let proxy: ProxyKubeApi = match data
        .get_object_from_redis(crd::REDIS_PREFIX, &format!("{ns}/{cluster}"))
        .await
    {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            error!(error = %e, "couldn't get proxy from redis");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    if !proxy.spec.enabled
        || proxy
            .spec
            .auth_config
            .as_ref()
            .is_some_and(|auth_config| !auth_config.oidc_provider.enabled)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Before any OIDC work: `oidc_core()` below performs an uncached discovery
    // fetch against the provider, and this endpoint is unauthenticated, so
    // without a gate here every anonymous request with a junk `state` turns into
    // an outbound request to the cluster's IdP. The CSRF check that rejects such
    // a request is further down, so the cheap rejection would otherwise sit
    // behind the expensive call. Same helper, same position, as the `/oauth/*`
    // siblings.
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }
    let (redirect_front, redirect_kubectl) = super::callback_target(&req);
    let oidc_conf = match proxy
        .get_oidc_conf(data.clone(), redirect_front, redirect_kubectl)
        .await
    {
        Ok(Some(conf)) => conf,
        Ok(None) => {
            error!("OIDC config not found or invalid");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(e) => {
            error!(error = %e, "couldn't resolve the OIDC config");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let client_reqwest = match oidc_conf.oidc_reqwest_client() {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't build oidc http client");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let client_oidc = match oidc_conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get oidc client");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    info!(%ns, %cluster, "Callback received for cluster");
    // The CSRF state/nonce is single-use: it is read and deleted atomically so
    // the same `code`+`state` cannot be replayed against the callback within its
    // TTL window, even by concurrent requests.
    let login_to_callback = match data
        .redis_take(&format!(
            "oidc_csrf_nonce:{}/{}/{}",
            ns, cluster, callback.state
        ))
        .await
    {
        Ok(Some(nonce)) => {
            info!("Nonce found in redis");
            if let Some(model) = LoginToCallBackModel::from_string(&nonce) {
                model
            } else {
                error!("Couldn't parse nonce object");
                return (StatusCode::BAD_REQUEST, "Invalid state").into_response();
            }
        }
        Ok(None) => {
            error!("Nonce not found in redis");
            return (StatusCode::BAD_REQUEST, "Invalid state").into_response();
        }
        Err(e) => {
            error!(error = %e, "couldn't get nonce");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let exchange_code =
        match client_oidc.exchange_code(AuthorizationCode::new(callback.code.clone())) {
            Ok(code) => code,
            Err(e) => {
                error!(error = %e, "couldn't exchange code");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    let token_response = match exchange_code
        // Set the PKCE code verifier.
        .set_pkce_verifier(login_to_callback.pkce_verifier())
        .request_async(&client_reqwest)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            error!(error = %e, "couldn't get token response");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let id_token = if let Some(id_token) = token_response.id_token() {
        id_token
    } else {
        error!("No ID token received");
        return (StatusCode::INTERNAL_SERVER_ERROR, "No ID token received").into_response();
    };
    let id_token_verifier = client_oidc.id_token_verifier();
    let claims = match id_token.claims(&id_token_verifier, &login_to_callback.nonce()) {
        Ok(claims) => claims,
        Err(e) => {
            error!(error = %e, "couldn't verify ID token");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Some(expected_access_token_hash) = claims.access_token_hash() {
        let signing_alg = match id_token.signing_alg() {
            Ok(alg) => alg,
            Err(e) => {
                error!(error = %e, "couldn't get signing alg");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let signing_key = match id_token.signing_key(&id_token_verifier) {
            Ok(key) => key,
            Err(e) => {
                error!(error = %e, "couldn't get signing key");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let actual_access_token_hash = match AccessTokenHash::from_token(
            token_response.access_token(),
            signing_alg,
            signing_key,
        ) {
            Ok(hash) => hash,
            Err(e) => {
                error!(error = %e, "couldn't get access token hash");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        if actual_access_token_hash != *expected_access_token_hash {
            return (StatusCode::BAD_REQUEST, "Invalid access token").into_response();
        }
    }
    let callback_body = CallbackModel {
        id_token: id_token.to_string(),
        access_token: token_response.access_token().secret().clone(),
        refresh_token: match token_response.refresh_token() {
            Some(refresh_token) => refresh_token.secret().clone(),
            None => String::new(),
        },
        cluster_url: proxy.to_full_path(data.clone()),
        subject: claims.subject().to_string(),
    };
    (StatusCode::OK, Json(callback_body)).into_response()
}
