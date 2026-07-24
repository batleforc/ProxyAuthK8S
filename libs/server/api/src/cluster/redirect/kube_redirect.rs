use actix_web::{dev::PeerAddr, http, web, HttpRequest, HttpResponse, Responder};
use common::State;
use crd::ProxyKubeApi;
use tracing::{debug, error, info, instrument, warn};

use crate::cluster::redirect::audit::AuditContext;
use crate::cluster::redirect::status_response::{forbidden, too_many_requests, unauthorized};
use crate::cluster::redirect::throttle;
use crate::helper::{extract_authorization_header, extract_ns_cluster};
use crate::model::user::User;

mod standard;
mod tls;
mod upgrade;
mod upstream;
mod virtual_redirect;

use standard::standard_redirect;
use upgrade::{is_upgrade_request, upgrade_redirect};
use virtual_api::MapperRegistry;
use virtual_redirect::virtual_redirect as serve_virtual_api;

/// Answer with `response`, recording the audit event for it first.
macro_rules! audited {
    ($audit:expr, $response:expr) => {{
        let response = $response;
        $audit.emit(response.status().as_u16());
        return response;
    }};
}

#[instrument(name = "main_redirect",fields(http.method= ?method, http.response.status_code) ,skip(req, data, payload))]
pub async fn redirect(
    req: HttpRequest,
    data: web::Data<State>,
    payload: web::Payload,
    method: http::Method,
    peer_addr: Option<PeerAddr>,
) -> impl Responder {
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

    if !proxy.spec.enabled {
        return HttpResponse::NotFound().finish();
    }

    // Only strip the routing prefix, never an occurrence further down the path:
    // `/clusters/{ns}/{cluster}` may legitimately reappear inside the upstream path.
    let prefix = format!("/clusters/{}/{}", ns, cluster);
    let upstream_path = match req.path().strip_prefix(&prefix) {
        Some(rest) => rest.to_string(),
        None => {
            error!(path = %req.path(), %prefix, "request path does not start with the cluster prefix");
            return HttpResponse::NotFound().finish();
        }
    };

    let mut audit = AuditContext::new(&ns, &cluster, method.as_str(), &upstream_path);

    let is_upgrade = is_upgrade_request(&req);

    debug!(proxy = ?proxy, "Proxy found for cluster");
    debug!(is_upgrade, "Is upgrade request");

    // Failed authentications are counted per client address: at that point no
    // user has been resolved, so the address is the only identity available.
    // Honour `TRUSTED_PROXY_COUNT` so that, behind an ingress, bans and
    // unauthenticated rate limits target the real client instead of the shared
    // ingress IP (which would let one attacker ban every user).
    let peer_ip = peer_addr.as_ref().map(|PeerAddr(addr)| addr.ip());
    let forwarded_for = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    let peer_id =
        crate::cluster::redirect::forwarded::throttle_client_ip(forwarded_for, peer_ip);

    if throttle::is_banned(data.get_ref(), &proxy, &peer_id).await {
        let retry_after = throttle::ban_retry_after(data.get_ref(), &proxy, &peer_id).await;
        let throttled = throttle::Throttled::Banned { retry_after };
        warn!(peer = %peer_id, "refusing a banned client");
        audited!(
            audit,
            too_many_requests(&throttled.message(), throttled.retry_after())
        );
    }

    let user = if proxy.need_token_validation() {
        let token = match extract_authorization_header(&req) {
            Ok(token) => token,
            Err(e) => {
                warn!("Authorization header extraction failed: {}", e);
                throttle::record_auth_failure(data.get_ref(), &proxy, &peer_id).await;
                audited!(audit, unauthorized("no usable bearer token was provided"));
            }
        };
        match User::get_user_info_with_proxy(
            data.get_ref().clone(),
            proxy.clone(),
            token.to_string(),
        )
        .await
        {
            Ok(Some(user)) => {
                throttle::clear_auth_failures(data.get_ref(), &proxy, &peer_id).await;
                Some(user)
            }
            Ok(None) => {
                warn!("User info not found in OIDC response");
                throttle::record_auth_failure(data.get_ref(), &proxy, &peer_id).await;
                audited!(
                    audit,
                    unauthorized("the token could not be resolved to a user")
                );
            }
            Err(e) => {
                warn!("Error while getting user info from OIDC token: {}", e);
                throttle::record_auth_failure(data.get_ref(), &proxy, &peer_id).await;
                audited!(audit, unauthorized("the token could not be validated"));
            }
        }
    } else {
        None
    };

    audit.with_user(user.as_ref());

    // Rate limiting is per user once one is known, so a shared NAT address does
    // not make several users share a budget.
    let rate_limit_subject = user
        .as_ref()
        .map(|user| user.username.clone())
        .unwrap_or_else(|| peer_id.clone());
    let user_groups: &[String] = user.as_ref().map(|u| u.groups.as_slice()).unwrap_or(&[]);
    if let Some(throttled) =
        throttle::check_rate_limit(data.get_ref(), &proxy, &rate_limit_subject, user_groups).await
    {
        audited!(
            audit,
            too_many_requests(&throttled.message(), throttled.retry_after())
        );
    }

    // Per-cluster authorization. A cluster restricted to a group can only be
    // reached by a caller we were able to authenticate.
    if let Some(proxy_group) = proxy.get_proxy_group() {
        match &user {
            Some(user) if proxy.is_proxy_allowed(&user.groups) => {}
            Some(user) => {
                warn!(
                    user = %user.username,
                    %proxy_group,
                    "user is not a member of the group required by this cluster"
                );
                audited!(
                    audit,
                    forbidden(&format!(
                        "access to this cluster requires membership of the \"{}\" group",
                        proxy_group
                    ))
                );
            }
            None => {
                // The cluster is group-restricted but nothing validates tokens,
                // so the group can never be checked: refuse rather than pretend.
                error!(
                    %proxy_group,
                    "cluster requires a group but token validation is disabled; refusing the request"
                );
                audited!(
                    audit,
                    forbidden("this cluster requires an authenticated user but token validation is disabled")
                );
            }
        }
    }

    // Virtual APIs are resolved before anything is forwarded, but the registry
    // is only built when the cluster declares one, so an ordinary cluster pays
    // nothing for the feature.
    let registry = if proxy.spec.virtual_apis.is_empty() {
        MapperRegistry::new()
    } else {
        MapperRegistry::from_kinds(&proxy.enabled_virtual_apis())
    };
    let virtual_plan = virtual_redirect::plan(&registry, &upstream_path, method.as_str());

    // Resource allow-list. An empty or disabled configuration allows everything.
    // When a virtual API rewrites the request, the path that is really reached
    // on the cluster is checked too: allowing a virtual path must not become a
    // way around the allow-list on the API it maps to.
    if let Some(security_config) = &proxy.spec.security_config {
        let username = user.as_ref().map(|u| u.username.as_str()).unwrap_or("");
        let groups: &[String] = user.as_ref().map(|u| u.groups.as_slice()).unwrap_or(&[]);

        let mut checked_paths = vec![upstream_path.as_str()];
        if let Some(mapped) = virtual_plan.as_ref().and_then(|plan| plan.upstream_path()) {
            checked_paths.push(mapped);
        }

        for path in checked_paths {
            if !security_config.is_path_allowed(path, username, groups) {
                warn!(%path, "request path is not in the allowed resources");
                audited!(
                    audit,
                    forbidden(&format!("path \"{}\" is not allowed on this cluster", path))
                );
            }
        }
    }

    let base_url = match proxy
        .spec
        .service
        .url_to_call(data.client.clone(), "default".to_string())
        .await
    {
        Ok(url) => url.trim_end_matches('/').to_string(),
        Err(err) => {
            error!(err, " couldn't get url to call");
            audited!(audit, HttpResponse::NotFound().finish());
        }
    };

    if let Some(virtual_plan) = virtual_plan {
        info!(from = %req.uri().to_string(), method = %method.as_str(), "Serving a virtual API request");
        return serve_virtual_api(
            req,
            data,
            payload,
            method,
            peer_addr,
            proxy,
            base_url,
            user,
            audit,
            registry,
            upstream_path,
            virtual_plan,
        )
        .await;
    }

    let url_to_call = {
        let query_string = req.query_string();
        if query_string.is_empty() {
            format!("{}{}", base_url, upstream_path)
        } else {
            format!("{}{}?{}", base_url, upstream_path, query_string)
        }
    };

    info!(from = %req.uri().to_string(), to = %url_to_call, method = %method.as_str(),
        "Forwarding request from {} to {} with method {}",
        req.uri().to_string(),
        url_to_call,
        method.as_str()
    );

    if is_upgrade {
        return upgrade_redirect(
            req,
            data,
            payload,
            method,
            peer_addr,
            proxy,
            url_to_call,
            user,
            audit,
        )
        .await;
    }

    standard_redirect(
        req,
        data,
        payload,
        method,
        peer_addr,
        proxy,
        url_to_call,
        user,
        audit,
    )
    .await
}
