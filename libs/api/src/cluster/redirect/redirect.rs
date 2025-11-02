use actix_web::{web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use deadpool_redis::redis::AsyncTypedCommands;
use tracing::{error, instrument};

#[instrument(name = "main_redirect", skip(data))]
pub async fn redirect(req: HttpRequest, data: web::Data<State>) -> impl Responder {
    let ns: String = req.match_info().get("ns").unwrap().parse().unwrap();
    let cluster: String = req.match_info().get("cluster").unwrap().parse().unwrap();
    let path: String = req.match_info().get("path").unwrap().parse().unwrap();
    let mut conn = match data.get_redis_conn().await {
        Ok(conn) => conn,
        Err(e) => return HttpResponse::ServiceUnavailable().body(e.to_string()),
    };
    let proxy_json = match conn.get(format!("proxyk8sauth:{}/{}", ns, cluster)).await {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(e) => return HttpResponse::ServiceUnavailable().body(e.to_string()),
    };

    let proxy = match ProxyKubeApi::from_json(&proxy_json) {
        Some(proxy) => proxy,
        None => {
            error!("Couldn't parse object");
            return HttpResponse::NotFound().finish();
        }
    };

    let url_to_call = match proxy
        .spec
        .service
        .url_to_call(data.client.clone(), "default".to_string())
        .await
    {
        Ok(url) => url,
        Err(err) => {
            error!(err);
            return HttpResponse::NotFound().finish();
        }
    };
    // https://github.com/actix/examples/blob/master/http-proxy/src/main.rs#L56
    HttpResponse::Ok().body(format!("{}/{}", url_to_call, path))
}
