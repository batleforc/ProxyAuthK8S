use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncTypedCommands;
use openidconnect::{core::CoreAuthenticationFlow, CsrfToken, Nonce, PkceCodeChallenge, Scope};
use tracing::{error, info, instrument};

use crate::{cluster::auth::auth_model::LoginToCallBackModel, model::user::User};

/// Redirect to the cluster's login page
///
/// If the cluster is not found or disabled, return 404
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster", body = String),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    security(
        ("oauth2" = [])
    ),
    params(
        ("ns", description = "Namespace"),
        ("cluster", description = "Cluster name"),
        ("x-front-callback" = String, Header, nullable, description = "If it's from the frontend, this header will be set."),
    )
)]
#[get("/{ns}/{cluster}/auth/login")]
#[instrument(name = "cluster_login", skip(data))]
pub async fn cluster_login(req: HttpRequest, data: web::Data<State>, user: User) -> impl Responder {
    let ns: String = req.match_info().get("ns").unwrap().parse().unwrap();
    let cluster: String = req.match_info().get("cluster").unwrap().parse().unwrap();
    let mut conn = match data.get_redis_conn().await {
        Ok(conn) => conn,
        Err(e) => {
            error!(error = %e, " couldn't get redis connection");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };
    let proxy_json = match conn.get(format!("proxyk8sauth:{}/{}", ns, cluster)).await {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => {
            error!(error = %e, " couldn't get proxy from redis");
            return HttpResponse::ServiceUnavailable().body(e.to_string());
        }
    };
    let proxy = match ProxyKubeApi::from_json(&proxy_json) {
        Some(proxy) => proxy,
        None => {
            error!("Couldn't parse object");
            return HttpResponse::NotFound().finish();
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
    let oidc_conf = match proxy.get_oidc_conf(data.into_inner(), redirect_front) {
        Some(conf) => conf,
        None => {
            error!("OIDC config not found");
            return HttpResponse::InternalServerError().finish();
        }
    };
    info!(
        "User {:?} is logging in to cluster {:?}",
        user.username, oidc_conf.redirect_url
    );
    let client = match oidc_conf.get_oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, " couldn't get oidc client");
            return HttpResponse::InternalServerError().body(e.to_string());
        }
    };
    let scopes: Vec<Scope> = oidc_conf
        .scopes
        .split(" ")
        .map(|s| Scope::new(s.to_string()))
        .collect();
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    // TODO : Finish implementing the login redirect.
    let (auth_url, csrf_token, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(pkce_challenge)
        .add_scopes(scopes)
        .url();

    // Store the csrf token and nonce in redis with a short TTL to validate later. TTL could be 5 minutes.
    match conn
        .set_ex::<String, String>(
            format!("oidc_csrf_nonce:{}/{}/{}", ns, cluster, csrf_token.secret()),
            LoginToCallBackModel::new(
                nonce.secret().to_string(),
                pkce_verifier.secret().to_string(),
            )
            .to_string(), // Replace with actual csrf and nonce values
            300,
        )
        .await
    {
        Ok(_) => (),
        Err(e) => {
            error!(error = %e, " couldn't store csrf and nonce in redis");
            return HttpResponse::InternalServerError().finish();
        }
    };

    HttpResponse::Ok().body(auth_url.to_string())
}
