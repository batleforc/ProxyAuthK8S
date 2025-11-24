use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncTypedCommands;
use tracing::{error, info, instrument};

use crate::model::user::User;

/// Redirect to the cluster's login page
///
/// If the cluster is not found or disabled, return 404
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster"),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 500, description = "Internal server error."),
    ),
    security(
        ("oauth2" = [])
    ),
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
    let oidc_conf = match proxy.get_oidc_conf(data.into_inner()) {
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
    // TODO : Finish implementing the login redirect
    HttpResponse::Ok().body("TODO: implement login redirect")
}
