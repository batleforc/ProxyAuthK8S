use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use openidconnect::{core::CoreAuthenticationFlow, CsrfToken, Nonce, PkceCodeChallenge, Scope};
use tracing::{error, info, instrument};

use crate::{
    cluster::auth::auth_model::LoginToCallBackModel, helper::extract_ns_cluster, model::user::User,
};

/// Redirect to the cluster's login page
///
/// If the cluster is not found or disabled, return 404.
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster.", body = String),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    security(
        ("bearer_auth" = [])
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
        ("x-front-callback" = String, Header, nullable, description = "If it's from the frontend, this header will be set."),
        ("x-kubectl-callback" = String, Header, nullable, description = "If it's from kubectl plugin, this header will be set."),
    )
)]
#[get("/{ns}/{cluster}/auth/login")]
#[instrument(name = "cluster_login", skip(data))]
pub async fn cluster_login(req: HttpRequest, data: web::Data<State>, user: User) -> impl Responder {
    /// The CSRF token and nonce only need to survive the redirect round-trip.
    const CSRF_NONCE_TTL_SECONDS: u64 = 300;

    let (ns, cluster) = match extract_ns_cluster(&req) {
        Some(parts) => parts,
        None => {
            error!(path = %req.path(), "missing ns/cluster path parameters");
            return HttpResponse::NotFound().finish();
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
        .map(|v| v.to_str().unwrap_or_default().to_string());
    let oidc_conf =
        match proxy.get_oidc_conf(data.clone().into_inner(), redirect_front, redirect_kubectl) {
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
            return HttpResponse::InternalServerError().finish();
        }
    };
    let scopes: Vec<Scope> = oidc_conf
        .scopes
        .split(" ")
        .map(|s| Scope::new(s.to_string()))
        .collect();
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token, nonce) = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(pkce_challenge)
        .add_scopes(scopes)
        .url();

    // Store the csrf token and nonce in redis with a short TTL to validate later.
    let login_to_callback = LoginToCallBackModel::new(
        nonce.secret().to_string(),
        pkce_verifier.secret().to_string(),
    );
    if let Err(e) = data
        .redis_set(
            &format!("oidc_csrf_nonce:{}/{}/{}", ns, cluster, csrf_token.secret()),
            &login_to_callback.to_string(),
            Some(CSRF_NONCE_TTL_SECONDS),
        )
        .await
    {
        error!(error = %e, " couldn't store csrf and nonce in redis");
        return HttpResponse::InternalServerError().finish();
    }

    HttpResponse::Ok().body(auth_url.to_string())
}
