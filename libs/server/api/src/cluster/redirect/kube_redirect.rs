use std::sync::Arc;

use axum::{
    extract::{OriginalUri, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use crd::ProxyKubeApi;
use crd::security::PortPolicy;
use crd::service::Service;
use tracing::{debug, error, info, instrument, warn};

use crate::cluster::redirect::audit::{AuditContext, PortDecision};
use crate::cluster::redirect::status_response::{
    agent_unavailable, bad_request, forbidden, too_many_requests, unauthorized,
};
use crate::cluster::redirect::throttle;
use crate::helper::{extract_authorization_header, forwarded_for, peer_ip};
use crate::model::user::User;

mod context;
mod exec;
mod list_fallback;
mod port_forward;
mod spdy_dictionary;
mod standard;
mod tls;
mod upgrade;
mod upstream;
mod virtual_redirect;
mod websocket;

use context::RedirectContext;
// Public for the `fuzz_port_target` fuzz target.
pub use exec::{ExecChannelFilter, parse_exec_query};
pub use port_forward::{PortTarget, port_target};
use standard::standard_redirect;
use upgrade::{is_upgrade_request, upgrade_redirect};
use virtual_api::MapperRegistry;
use virtual_redirect::virtual_redirect as serve_virtual_api;

/// Answer with `response`, recording the audit event for it first.
macro_rules! audited {
    ($audit:expr_2021, $response:expr_2021) => {{
        let response = $response;
        $audit.emit_refusal(response.status().as_u16());
        return response;
    }};
}

/// The proxy route: `ns` and `cluster` come from the router, the upstream
/// path from the raw URI (never percent-decoded, see RFC 0007 T3).
#[instrument(name = "main_redirect", fields(http.method = ?req.method(), http.response.status_code), skip(req, data))]
pub async fn redirect(
    data: Arc<common::State>,
    ns: String,
    cluster: String,
    req: Request,
) -> Response {
    let (mut req, payload) = req.into_parts();
    // Nested under `/clusters`, the router hands down the URI without that
    // prefix; the original one is what every check below reads, raw.
    if let Some(OriginalUri(original)) = req.extensions.get::<OriginalUri>().cloned() {
        req.uri = original;
    }
    let method = req.method.clone();

    let proxy: ProxyKubeApi = match data
        .get_object_from_redis(crd::REDIS_PREFIX, &format!("{ns}/{cluster}"))
        .await
    {
        Ok(Some(proxy)) => proxy,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            error!(error = %e, "couldn't get proxy from redis");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    if !proxy.spec.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Only strip the routing prefix, never an occurrence further down the path:
    // `/clusters/{ns}/{cluster}` may legitimately reappear inside the upstream path.
    let prefix = format!("/clusters/{ns}/{cluster}");
    let upstream_path = if let Some(rest) = req.uri.path().strip_prefix(&prefix) {
        rest.to_string()
    } else {
        error!(path = %req.uri.path(), %prefix, "request path does not start with the cluster prefix");
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut audit = AuditContext::new(&ns, &cluster, method.as_str(), &upstream_path);
    // The Kubernetes verb, computed as the apiserver does (RFC 0006): every
    // audit line of this request carries it, refusals included.
    let request_info = crd::security::request_info(
        method.as_str(),
        &upstream_path,
        req.uri.query().unwrap_or_default(),
    )
    .ok();
    audit.k8s_verb = request_info
        .as_ref()
        .map(|info| info.verb.as_str().to_string());
    // What an exec or attach runs goes to every audit line of the request
    // (RFC 0004); an unreadable query is left to the policy of phase 2.
    if exec::is_exec_or_attach_path(&upstream_path)
        && let Ok(request) = exec::parse_exec_query(req.uri.query().unwrap_or_default())
    {
        let mask = proxy
            .spec
            .security_config
            .as_ref()
            .is_some_and(|config| config.mask_exec_secrets);
        audit.exec = Some(crate::cluster::redirect::audit::ExecAudit {
            argv: if mask {
                exec::mask_secrets(&request.argv)
            } else {
                request.argv
            },
            container: request.container,
            stdin: request.stdin,
            tty: request.tty,
        });
    }

    let is_upgrade = is_upgrade_request(&req, &upstream_path);

    debug!(proxy = ?proxy, "Proxy found for cluster");
    debug!(is_upgrade, "Is upgrade request");

    // Failed authentications are counted per client address: at that point no
    // user has been resolved, so the address is the only identity available.
    // Honour `TRUSTED_PROXY_COUNT` so that, behind an ingress, bans and
    // unauthenticated rate limits target the real client instead of the shared
    // ingress IP (which would let one attacker ban every user).
    let peer_ip = peer_ip(&req);
    let peer_id = crate::cluster::redirect::forwarded::throttle_client_ip(
        forwarded_for(&req.headers),
        peer_ip,
    );

    if throttle::is_banned(&data, &proxy, &peer_id).await {
        let retry_after = throttle::ban_retry_after(&data, &proxy, &peer_id).await;
        let throttled = throttle::Throttled::Banned { retry_after };
        warn!(peer = %peer_id, "refusing a banned client");
        audited!(
            audit,
            too_many_requests(&throttled.message(), throttled.retry_after())
        );
    }

    let user = if proxy.need_token_validation() {
        let token = match extract_authorization_header(&req.headers) {
            Ok(token) => token,
            Err(e) => {
                warn!("Authorization header extraction failed: {}", e);
                throttle::record_auth_failure(&data, &proxy, &peer_id).await;
                audited!(audit, unauthorized("no usable bearer token was provided"));
            }
        };
        match User::get_user_info_with_proxy((*data).clone(), proxy.clone(), token.to_string())
            .await
        {
            Ok(Some(user)) => {
                throttle::clear_auth_failures(&data, &proxy, &peer_id).await;
                Some(user)
            }
            Ok(None) => {
                warn!("User info not found in OIDC response");
                throttle::record_auth_failure(&data, &proxy, &peer_id).await;
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
                    throttle::record_auth_failure(&data, &proxy, &peer_id).await;
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
        throttle::check_rate_limit(&data, &proxy, &rate_limit_subject, user_groups).await
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

    // Resource allow-list, on the path and the Kubernetes verb (computed as the
    // apiserver does). An empty or disabled configuration allows everything.
    // When a virtual API rewrites the request, the path that is really reached
    // on the cluster is checked too: allowing a virtual path must not become a
    // way around the allow-list on the API it maps to.
    let mapped = virtual_plan
        .as_ref()
        .and_then(|plan| plan.upstream_path())
        .map(|mapped| {
            let info = crd::security::request_info(
                method.as_str(),
                mapped,
                req.uri.query().unwrap_or_default(),
            )
            .ok();
            (mapped, info)
        });
    if let Some((path, info)) = &mapped {
        audit.upstream = Some((
            (*path).to_string(),
            info.as_ref().map(|info| info.verb.as_str().to_string()),
        ));
    }
    if let Some(security_config) = &proxy.spec.security_config {
        let username = user.as_ref().map_or("", |u| u.username.as_str());
        let groups: &[String] = user.as_ref().map(|u| u.groups.as_slice()).unwrap_or(&[]);

        let mut checked = vec![(upstream_path.as_str(), request_info.clone())];
        checked.extend(mapped.clone());

        for (path, info) in checked {
            if security_config.is_request_allowed(path, info.as_ref(), username, groups) {
                continue;
            }
            if security_config.matches_path(path, username, groups) {
                let verb = info
                    .as_ref()
                    .map_or(method.as_str(), |info| info.verb.as_str());
                warn!(%path, %verb, "request verb is not in the allowed resources");
                audit.emit_verb_denied();
                audited!(
                    audit,
                    forbidden(&format!("verb \"{verb}\" is not allowed on \"{path}\""))
                );
            }
            warn!(%path, "request path is not in the allowed resources");
            audited!(
                audit,
                forbidden(&format!("path \"{path}\" is not allowed on this cluster"))
            );
        }
    }

    // Port policy, evaluated before any relay whatever the transport: every
    // path that names a port goes through `port_target` once. A port-forward
    // takes the policy along (its ports are in the query or the SPDY frames).
    // `nodes/proxy` reaches the kubelet's own exec and port-forward as a `get`,
    // so a port or a verb restriction closes it.
    let port_policy = match &proxy.spec.security_config {
        Some(security_config) => {
            let username = user.as_ref().map_or("", |u| u.username.as_str());
            let groups: &[String] = user.as_ref().map(|u| u.groups.as_slice()).unwrap_or(&[]);
            let info = request_info.as_ref();
            let target = port_forward::port_target(&upstream_path);
            if target == PortTarget::NodeProxy
                && security_config.restricts_node_proxy(username, groups)
                && !security_config.node_proxy_explicitly_allowed(
                    &upstream_path,
                    info,
                    username,
                    groups,
                )
            {
                let reason = "nodes/proxy is not allowed while ports, verbs or exec are restricted";
                warn!(path = %upstream_path, %reason, "refusing a request outside the port policy");
                audit.emit_port_decision(PortDecision::NodeProxyDenied, None);
                audited!(audit, forbidden(reason));
            }
            if security_config.restricts_ports(username, groups) {
                let policy = security_config.port_policy(&upstream_path, info, username, groups);
                let refusal = match target {
                    PortTarget::ProxyPort(port) if policy.allows(port) => {
                        audit.emit_port_decision(PortDecision::Allowed, Some(port));
                        None
                    }
                    PortTarget::ProxyPort(port) => Some((
                        Some(port),
                        format!("port {port} is not allowed on this cluster"),
                    )),
                    PortTarget::ProxyNamedOrMissing => Some((
                        None,
                        "named or missing port is not allowed on this cluster".to_string(),
                    )),
                    PortTarget::NodeProxy | PortTarget::PortForward | PortTarget::None => None,
                };
                if let Some((port, reason)) = refusal {
                    warn!(path = %upstream_path, %reason, "refusing a request outside the port policy");
                    audit.emit_port_decision(PortDecision::Denied, port);
                    audited!(audit, forbidden(&reason));
                }
                policy
            } else {
                PortPolicy::Any
            }
        }
        None => PortPolicy::Any,
    };

    // Exec policy (RFC 0004), on the query, before any byte reaches the
    // cluster and before the websocket-only check: kubectl's SPDY fallback
    // gets the same refusal and its reason.
    let mut exec_streams = None;
    if exec::is_exec_or_attach_path(&upstream_path)
        && let Some(security_config) = &proxy.spec.security_config
    {
        let username = user.as_ref().map_or("", |u| u.username.as_str());
        let groups: &[String] = user.as_ref().map(|u| u.groups.as_slice()).unwrap_or(&[]);
        let rules =
            security_config.exec_rules(&upstream_path, request_info.as_ref(), username, groups);
        if matches!(rules, crd::security::ExecRules::Restricted(_)) {
            let request = match exec::parse_exec_query(req.uri.query().unwrap_or_default()) {
                Ok(request) => request,
                Err(reason) => {
                    warn!(path = %upstream_path, %reason, "refusing an unreadable exec query");
                    audited!(audit, bad_request(&reason));
                }
            };
            if let Err(denial) = rules.check(&request, exec::is_attach_path(&upstream_path)) {
                warn!(path = %upstream_path, reason = denial.reason, "refusing an exec outside the policy");
                audit.emit_exec_decision(Some(denial.reason));
                audited!(audit, forbidden(&denial.message));
            }
            audit.emit_exec_decision(None);
            exec_streams = Some((request.stdin, request.tty));
        }
    }

    // RFC 0012: an `Agent` cluster has no address the proxy may dial, only a
    // gateway pod holding its tunnel. Without one it is refused here, never
    // reached directly; the lookup also warms the route reqwest picks.
    if matches!(proxy.spec.service, Service::Agent(_)) {
        let routed = match proxy.metadata.uid.as_deref() {
            Some(uid) => data.agent_gateway.route(&data, uid).await.is_ok(),
            None => false,
        };
        if !routed {
            audited!(audit, agent_unavailable());
        }
    }

    // A `KubernetesService` without an explicit namespace lives next to the
    // proxy resource, exactly as the reachability check and the kubeconfig
    // export resolve it. `ns` is the namespace the proxy was looked up in.
    let base_url = match proxy
        .spec
        .service
        .url_to_call(data.client.clone(), ns.to_string())
        .await
    {
        Ok(url) => url.trim_end_matches('/').to_string(),
        Err(err) => {
            error!(error = %err, "couldn't get url to call");
            audited!(audit, StatusCode::NOT_FOUND.into_response());
        }
    };

    let ctx = RedirectContext {
        req,
        body: payload,
        data,
        method,
        peer_ip,
        proxy,
        user,
        audit,
        base_url,
        upstream_path,
    };
    if let Some(virtual_plan) = virtual_plan {
        // Path only at info: query strings can carry user-controlled or
        // sensitive values (label selectors, field selectors, exec commands).
        info!(from = %ctx.req.uri.path(), method = %ctx.method.as_str(), "Serving a virtual API request");
        debug!(from = %ctx.req.uri, "Virtual API request full URI");
        return serve_virtual_api(ctx, registry, virtual_plan).await;
    }

    // Path only at info (no query string: it can carry user-controlled or
    // sensitive values such as exec commands or selectors); the full URLs are
    // kept at debug for troubleshooting.
    info!(from = %ctx.req.uri.path(), to = %ctx.url_without_query(),
        method = %ctx.method.as_str(), "Forwarding request");
    debug!(from = %ctx.req.uri, to = %ctx.url_to_call(), "Forwarding request (full URLs)");

    if is_upgrade {
        return upgrade_redirect(ctx, port_policy, exec_streams).await;
    }

    standard_redirect(ctx).await
}
