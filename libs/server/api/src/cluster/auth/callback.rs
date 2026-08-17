use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use common::State;
use crd::ProxyKubeApi;
use crd_runtime::ProxyKubeApiRuntime;
use openidconnect::{AccessTokenHash, AuthorizationCode, OAuth2TokenResponse, TokenResponse};
use serde::Deserialize;
use tracing::{error, info, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::{auth_model::LoginToCallBackModel, callback_model::CallbackModel};
use crate::helper::extract_ns_cluster;

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
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster.", body = CallbackModel),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
        ("x-front-callback" = String, Header, nullable, description = "If it's from the frontend, this header will be set."),
        ("x-kubectl-callback" = String, Header, nullable, description = "If it's from kubectl plugin, this header will be set."),
        CallbackQuery,
    )
)]
#[get("/{ns}/{cluster}/auth/callback")]
#[instrument(name = "cluster_callback", skip(data, callback))]
pub async fn callback_login(
    req: HttpRequest,
    data: web::Data<State>,
    callback: web::Query<CallbackQuery>,
) -> impl Responder {
    let (ns, cluster) = if let Some(parts) = extract_ns_cluster(&req) {
        parts
    } else {
        error!(path = %req.path(), "missing ns/cluster path parameters");
        return HttpResponse::NotFound().finish();
    };

    let proxy: ProxyKubeApi = match data
        .get_object_from_redis(crd::REDIS_PREFIX, &format!("{ns}/{cluster}"))
        .await
    {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => {
            error!(error = %e, "couldn't get proxy from redis");
            return HttpResponse::ServiceUnavailable().finish();
        }
    };

    if !proxy.spec.enabled
        || proxy
            .spec
            .auth_config
            .as_ref()
            .is_some_and(|auth_config| !auth_config.oidc_provider.enabled)
    {
        return HttpResponse::NotFound().finish();
    }
    let redirect_front = req.headers().contains_key("x-front-callback");
    let redirect_kubectl = req
        .headers()
        .get("x-kubectl-callback")
        .and_then(|v| v.to_str().ok())
        .map(std::string::ToString::to_string);
    let oidc_conf = if let Some(conf) =
        proxy.get_oidc_conf(data.clone().into_inner(), redirect_front, redirect_kubectl)
    {
        conf
    } else {
        error!("OIDC config not found or invalid");
        return HttpResponse::InternalServerError().finish();
    };
    let client_reqwest = match oidc_conf.oidc_reqwest_client() {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't build oidc http client");
            return HttpResponse::InternalServerError().finish();
        }
    };
    let client_oidc = match oidc_conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get oidc client");
            return HttpResponse::InternalServerError().finish();
        }
    };
    info!(%ns, %cluster, "Callback received for cluster");
    let login_to_callback = match data
        .redis_get(&format!(
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
                return HttpResponse::BadRequest().body("Invalid state");
            }
        }
        Ok(None) => {
            error!("Nonce not found in redis");
            return HttpResponse::BadRequest().body("Invalid state");
        }
        Err(e) => {
            error!(error = %e, "couldn't get nonce");
            return HttpResponse::ServiceUnavailable().finish();
        }
    };
    // The CSRF state/nonce is single-use: drop it now so the same `code`+`state`
    // cannot be replayed against the callback within its TTL window.
    if let Err(e) = data
        .delete_key(&format!(
            "oidc_csrf_nonce:{}/{}/{}",
            ns, cluster, callback.state
        ))
        .await
    {
        error!(error = %e, "couldn't delete used csrf state");
    }
    let exchange_code =
        match client_oidc.exchange_code(AuthorizationCode::new(callback.code.clone())) {
            Ok(code) => code,
            Err(e) => {
                error!(error = %e, "couldn't exchange code");
                return HttpResponse::InternalServerError().finish();
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
            return HttpResponse::InternalServerError().finish();
        }
    };

    let id_token = if let Some(id_token) = token_response.id_token() {
        id_token
    } else {
        error!("No ID token received");
        return HttpResponse::InternalServerError().body("No ID token received");
    };
    let id_token_verifier = client_oidc.id_token_verifier();
    let claims = match id_token.claims(&id_token_verifier, &login_to_callback.nonce()) {
        Ok(claims) => claims,
        Err(e) => {
            error!(error = %e, "couldn't verify ID token");
            return HttpResponse::InternalServerError().finish();
        }
    };

    if let Some(expected_access_token_hash) = claims.access_token_hash() {
        let signing_alg = match id_token.signing_alg() {
            Ok(alg) => alg,
            Err(e) => {
                error!(error = %e, "couldn't get signing alg");
                return HttpResponse::InternalServerError().finish();
            }
        };
        let signing_key = match id_token.signing_key(&id_token_verifier) {
            Ok(key) => key,
            Err(e) => {
                error!(error = %e, "couldn't get signing key");
                return HttpResponse::InternalServerError().finish();
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
                return HttpResponse::InternalServerError().finish();
            }
        };
        if actual_access_token_hash != *expected_access_token_hash {
            return HttpResponse::BadRequest().body("Invalid access token");
        }
    }
    let callback_body = CallbackModel {
        id_token: id_token.to_string(),
        access_token: token_response.access_token().secret().clone(),
        refresh_token: match token_response.refresh_token() {
            Some(refresh_token) => refresh_token.secret().clone(),
            None => String::new(),
        },
        cluster_url: proxy.to_full_path(data.clone().into_inner()),
        subject: claims.subject().to_string(),
    };
    HttpResponse::Ok().json(callback_body)
}
