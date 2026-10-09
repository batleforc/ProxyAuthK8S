use actix_web::{HttpRequest, HttpResponse, Responder, dev::PeerAddr, http, web};
use common::State;
use crd::ProxyKubeApi;
use tracing::{debug, error, info, instrument, warn};

use crate::cluster::redirect::audit::AuditContext;
use crate::cluster::redirect::status_response::{forbidden, too_many_requests, unauthorized};
use crate::cluster::redirect::throttle;
use crate::helper::{extract_authorization_header, extract_ns_cluster};
use crate::model::user::User;

mod context;
mod list_fallback;
mod standard;
mod tls;
mod upgrade;
mod upstream;
mod virtual_redirect;

use context::RedirectContext;
use standard::standard_redirect;
use upgrade::{is_upgrade_request, upgrade_redirect};
use virtual_api::MapperRegistry;
use virtual_redirect::virtual_redirect as serve_virtual_api;

/// Answer with `response`, recording the audit event for it first.
macro_rules! audited {
    ($audit:expr_2021, $response:expr_2021) => {{
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

    if !proxy.spec.enabled {
        return HttpResponse::NotFound().finish();
    }

    // Only strip the routing prefix, never an occurrence further down the path:
    // `/clusters/{ns}/{cluster}` may legitimately reappear inside the upstream path.
    let prefix = format!("/clusters/{ns}/{cluster}");
    let upstream_path = if let Some(rest) = req.path().strip_prefix(&prefix) {
        rest.to_string()
    } else {
        error!(path = %req.path(), %prefix, "request path does not start with the cluster prefix");
        return HttpResponse::NotFound().finish();
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
    let peer_id = crate::cluster::redirect::forwarded::throttle_client_ip(forwarded_for, peer_ip);

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
                // Only a fault in the caller's own token counts toward a ban. A
                // server-side failure (an unreadable `config_from` Secret, a
                // Redis blip, an unreachable JWKS) is not something a different
                // token would have avoided, and charging it to the caller would
                // ban legitimate clients for an outage they did not cause.
                if e.is_caller_fault() {
                    throttle::record_auth_failure(data.get_ref(), &proxy, &peer_id).await;
                } else {
                    warn!("not counting a server-side auth failure against the caller");
                }
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
        .map_or_else(|| peer_id.clone(), |user| user.username.clone());
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
    if let Some(proxy_group) = proxy.proxy_group() {
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
                        "access to this cluster requires membership of the \"{proxy_group}\" group"
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
                    forbidden(
                        "this cluster requires an authenticated user but token validation is disabled"
                    )
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
        let username = user.as_ref().map_or("", |u| u.username.as_str());
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
                    forbidden(&format!("path \"{path}\" is not allowed on this cluster"))
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
            error!(error = %err, "couldn't get url to call");
            audited!(audit, HttpResponse::NotFound().finish());
        }
    };

    let from = req.uri().to_string();
    let ctx = RedirectContext {
        req,
        data,
        payload,
        method,
        peer_addr,
        proxy,
        user,
        audit,
        base_url,
        upstream_path,
    };

    if let Some(virtual_plan) = virtual_plan {
        info!(%from, method = %ctx.method.as_str(), "Serving a virtual API request");
        return serve_virtual_api(ctx, registry, virtual_plan).await;
    }

    let url_to_call = ctx.url_to_call();
    info!(%from, to = %url_to_call, method = %ctx.method.as_str(),
        "Forwarding request from {} to {} with method {}",
        from,
        url_to_call,
        ctx.method.as_str()
    );

    if is_upgrade {
        return upgrade_redirect(ctx).await;
    }

    standard_redirect(ctx).await
}
