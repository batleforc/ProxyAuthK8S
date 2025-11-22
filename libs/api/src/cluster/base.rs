use actix_web::{get, web, HttpRequest, HttpResponse, Responder};
use common::State;
use deadpool_redis::redis::AsyncTypedCommands;
use tracing::instrument;

/// Cluster base path
///
/// ATM only allow to know if cluster exist
#[utoipa::path(
    tag = "proxy_clusters",
    responses(
        (status = 200, description = "ATM nothing real"),
        (status = 500, description = "Internal server error."),
    )
)]
#[get("/{ns}/{cluster}")]
#[instrument(name = "base_cluster", skip(data))]
pub async fn base_cluster(req: HttpRequest, data: web::Data<State>) -> impl Responder {
    let ns: String = req.match_info().get("ns").unwrap().parse().unwrap();
    let cluster: String = req.match_info().get("cluster").unwrap().parse().unwrap();
    let mut conn = match data.get_redis_conn().await {
        Ok(conn) => conn,
        Err(e) => return HttpResponse::ServiceUnavailable().body(e.to_string()),
    };
    match conn.get(format!("proxyk8sauth:{}/{}", ns, cluster)).await {
        Ok(proxy_opt) => {
            if let Some(proxy) = proxy_opt {
                return HttpResponse::Ok().json(proxy);
            }
            HttpResponse::NotFound().finish()
        }
        Err(e) => HttpResponse::ServiceUnavailable().body(e.to_string()),
    }
}
