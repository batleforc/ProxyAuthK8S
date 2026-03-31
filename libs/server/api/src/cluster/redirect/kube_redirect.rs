use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use tracing::{debug, error, info, instrument};

use crate::model::user::User;

mod standard;
mod tls;
mod upgrade;

use standard::standard_redirect;
use upgrade::{is_upgrade_request, upgrade_redirect};

#[instrument(name = "main_redirect",fields(http.method= ?method, http.response.status_code) ,skip(req, data, payload))]
pub async fn redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
    let ns: String = req.match_info().get("ns").unwrap().parse().unwrap();
    let cluster: String = req.match_info().get("cluster").unwrap().parse().unwrap();

    // TODO: MTLS ? https://github.com/actix/examples/blob/master/https-tls/rustls-client-cert/src/main.rs

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

    if !proxy.spec.enabled {
        return HttpResponse::NotFound().finish();
    }

    let is_upgrade = is_upgrade_request(&req);

    debug!(proxy = ?proxy, "Proxy found for cluster");
    debug!(is_upgrade, "Is upgrade request");

    let _user = if proxy.need_token_validation() {
        if req.headers().clone().get("authorization").is_none() {
            return HttpResponse::Unauthorized().finish();
        }
        let token = match req
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
        {
            Some(token) => token,
            None => {
                tracing::warn!("Authorization header is not a valid string");
                return HttpResponse::Unauthorized().finish();
            }
        };
        match User::get_user_info_with_proxy(
            data.get_ref().clone(),
            proxy.clone(),
            token.to_string(),
        )
        .await
        {
            Ok(Some(user)) => Some(user),
            Ok(None) => {
                tracing::warn!("User info not found in OIDC response");
                return HttpResponse::Unauthorized().finish();
            }
            Err(e) => {
                tracing::warn!("Error while getting user info from OIDC token: {}", e);
                return HttpResponse::Unauthorized().finish();
            }
        }
    } else {
        None
    };

    // TODO: Check if user need to be validated for this cluster and if so, validate it before forwarding the request

    let url_to_call = match proxy
        .spec
        .service
        .url_to_call(data.client.clone(), "default".to_string())
        .await
    {
        Ok(url) => {
            let base_url = req
                .path()
                .to_string()
                .replace(&format!("/clusters/{}/{}", ns, cluster), &url);
            let query_string = req.query_string();
            if !query_string.is_empty() {
                format!("{}?{}", base_url, query_string)
            } else {
                base_url
            }
        }
        Err(err) => {
            error!(err, " couldn't get url to call");
            return HttpResponse::NotFound().finish();
        }
    };
    // TODO: Check if base url filters exist
    info!(from = %req.uri().to_string(), to = %url_to_call, method = %method.as_str(),
        "Forwarding request from {} to {} with method {}",
        req.uri().to_string(),
        url_to_call,
        method.as_str()
    );
    if is_upgrade {
        return upgrade_redirect(req, data, payload, method, peer_addr, proxy, url_to_call).await;
    }

    standard_redirect(req, data, payload, method, peer_addr, proxy, url_to_call).await
}
