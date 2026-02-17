use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncTypedCommands;
use openidconnect::{AccessTokenHash, AuthorizationCode, OAuth2TokenResponse, TokenResponse};
use serde::Deserialize;
use tracing::{error, info, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::{auth_model::LoginToCallBackModel, callback_model::CallbackModel};

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct CallbackQuery {
    /// Authorization code from the OIDC provider
    pub code: String,
    /// State parameter to prevent CSRF
    pub state: String,
}

/// Callback from the cluster's OIDC provider
///
/// If the cluster is not found or disabled, return 404
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster", body = CallbackModel),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace"),
        ("cluster" = String, description = "Cluster name"),
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
    let ns: String = req.match_info().get("ns").unwrap().parse().unwrap();
    let cluster: String = req.match_info().get("cluster").unwrap().parse().unwrap();
    let mut conn = match data.get_redis_conn().await {
        Ok(conn) => conn,
        Err(e) => {
            error!(error = %e, " couldn't get redis connection");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };

    let proxy: ProxyKubeApi = match data
        .get_object_from_redis("proxyk8sauth".to_string(), format!("{}/{}", ns, cluster))
        .await
    {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => {
            error!(error = %e, " couldn't get proxy from redis");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };

    if !proxy.spec.enabled
        || proxy.spec.clone().auth_config.is_some()
            && !proxy
                .spec
                .clone()
                .auth_config
                .unwrap()
                .oidc_provider
                .enabled
    {
        return HttpResponse::NotFound().finish();
    }
    let redirect_front = req.headers().contains_key("x-front-callback");
    let redirect_kubectl = req
        .headers()
        .get("x-kubectl-callback")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let oidc_conf =
        match proxy.get_oidc_conf(data.clone().into_inner(), redirect_front, redirect_kubectl) {
            Some(conf) => conf,
            None => {
                error!("OIDC config not found or invalid");
                return HttpResponse::InternalServerError().finish();
            }
        };
    let client_reqwest = oidc_conf.get_reqwest_client();
    let client_oidc = match oidc_conf.get_oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, " couldn't get oidc client");
            return HttpResponse::InternalServerError().body(e.to_string());
        }
    };
    info!(
        "Callback received for cluster {:?}",
        oidc_conf.client_secret
    );
    let login_to_callback = match conn
        .get::<String>(format!(
            "oidc_csrf_nonce:{}/{}/{}",
            ns,
            cluster,
            &callback.state.clone()
        ))
        .await
    {
        Ok(Some(nonce)) => {
            info!("Nonce found in redis");
            match LoginToCallBackModel::from_string(&nonce) {
                Some(model) => model,
                None => {
                    error!("Couldn't parse nonce object");
                    return HttpResponse::BadRequest().body("Invalid state");
                }
            }
        }
        Ok(None) => {
            error!("Nonce not found in redis");
            return HttpResponse::BadRequest().body("Invalid state");
        }
        Err(e) => {
            error!(error = %e, " couldn't get nonce");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };
    let exchange_code =
        match client_oidc.exchange_code(AuthorizationCode::new(callback.code.clone())) {
            Ok(code) => code,
            Err(e) => {
                error!(error = %e, " couldn't exchange code");
                return HttpResponse::InternalServerError().body(e.to_string());
            }
        };
    let token_response = match exchange_code
        // Set the PKCE code verifier.
        .set_pkce_verifier(login_to_callback.get_pkce_verifier())
        .request_async(&client_reqwest)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            error!(error = %e, " couldn't get token response");
            return HttpResponse::InternalServerError().body(e.to_string());
        }
    };

    let id_token = match token_response.id_token() {
        Some(id_token) => id_token,
        None => {
            error!("No ID token received");
            return HttpResponse::InternalServerError().body("No ID token received");
        }
    };
    let id_token_verifier = client_oidc.id_token_verifier();
    let claims = match id_token.claims(&id_token_verifier, &login_to_callback.get_nonce()) {
        Ok(claims) => claims,
        Err(e) => {
            error!(error = %e, " couldn't verify ID token");
            return HttpResponse::InternalServerError().finish();
        }
    };

    if let Some(expected_access_token_hash) = claims.access_token_hash() {
        let signing_alg = match id_token.signing_alg() {
            Ok(alg) => alg,
            Err(e) => {
                error!(error = %e, " couldn't get signing alg");
                return HttpResponse::InternalServerError().finish();
            }
        };
        let signing_key = match id_token.signing_key(&id_token_verifier) {
            Ok(key) => key,
            Err(e) => {
                error!(error = %e, " couldn't get signing key");
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
                error!(error = %e, " couldn't get access token hash");
                return HttpResponse::InternalServerError().finish();
            }
        };
        if actual_access_token_hash != *expected_access_token_hash {
            return HttpResponse::BadRequest().body("Invalid access token");
        }
    }
    let callback_body = CallbackModel {
        id_token: id_token.to_string(),
        access_token: token_response.access_token().secret().to_string(),
        refresh_token: match token_response.refresh_token() {
            Some(refresh_token) => refresh_token.secret().to_string(),
            None => "".to_string(),
        },
        cluster_url: proxy.to_full_path(data.clone().into_inner()),
        subject: claims.subject().to_string(),
    };
    HttpResponse::Ok().json(callback_body)
}
