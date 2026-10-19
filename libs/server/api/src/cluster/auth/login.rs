use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use crd::ProxyKubeApi;
use crd_runtime::ProxyKubeApiRuntime;
use openidconnect::{CsrfToken, Nonce, PkceCodeChallenge, Scope, core::CoreAuthenticationFlow};
use tracing::{error, info, instrument};

use crate::cluster::redirect::audit::{Event, HandlerAudit};
use crate::{
    cluster::auth::{auth_model::LoginToCallBackModel, throttle_oauth_as},
    cluster::redirect::{forwarded, throttle},
    helper::{NsCluster, extract_authorization_header, forwarded_for, peer_ip},
    model::user::User,
};

/// Redirect to the cluster's login page
///
/// If the cluster is not found or disabled, return 404.
#[utoipa::path(
    get,
    path = "/{ns}/{cluster}/auth/login",
    tag = "auth_clusters",
    responses(
        (status = 200, description = "Response from remote cluster.", body = String),
        (status = 401, description = "Missing, malformed, or unresolvable bearer token."),
        (status = 404, description = "Cluster not found or disabled."),
        (status = 429, description = "Rate limited or banned for this cluster."),
        (status = 500, description = "Internal server error."),
        (status = 503, description = "Redis is unavailable."),
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
#[instrument(name = "cluster_login", skip(data, req))]
pub async fn cluster_login(
    State(data): State<Arc<common::State>>,
    Path(NsCluster { ns, cluster }): Path<NsCluster>,
    req: Parts,
) -> Response {
    let mut audit = HandlerAudit::new(&ns, &cluster);
    let response = cluster_login_inner(data, ns, cluster, req, &mut audit).await;
    audit.finish(Event::AuthLogin, Event::AuthLogin, &response);
    response
}

/// The handler itself; [`cluster_login`] writes its audit line.
async fn cluster_login_inner(
    data: Arc<common::State>,
    ns: String,
    cluster: String,
    req: Parts,
    audit: &mut HandlerAudit,
) -> Response {
    /// The CSRF token and nonce only need to survive the redirect round-trip.
    const CSRF_NONCE_TTL_SECONDS: u64 = 300;

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
    if !proxy.spec.enabled
        || proxy
            .spec
            .auth_config
            .as_ref()
            .is_some_and(|auth_config| !auth_config.oidc_provider.enabled)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Authentication happens in the body rather than through the `User`
    // extractor. An extractor runs before the handler, so the proxy — and with
    // it this cluster's throttling policy — is not resolved yet, and the
    // extractor's own `/userinfo` call (preceded by an uncached discovery fetch)
    // would already have hit the IdP by the time any gate could fire. Doing it
    // here puts the ban check ahead of every outbound call and lets a failed
    // token feed fail2login, which the extractor's bare 401 never did.
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }
    let peer_id = forwarded::throttle_client_ip(forwarded_for(&req.headers), peer_ip(&req));

    let token = match extract_authorization_header(&req.headers) {
        Ok(token) => token,
        Err(e) => {
            error!("Authorization header extraction failed: {}", e);
            audit.deny("no usable bearer token");
            throttle::record_auth_failure(&data, &proxy, &peer_id).await;
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };
    // Validated against the proxy's own auth server (`state.oidc_client`), which
    // is what the `User` extractor used — the per-cluster provider below governs
    // the cluster login being started, not who may start it.
    let user = match User::get_user_info_from_oidc_token(
        token.to_string(),
        data.oidc_client.clone(),
    )
    .await
    {
        Ok(Some(user)) => {
            throttle::clear_auth_failures(&data, &proxy, &peer_id).await;
            audit.user(&user.username, &user.groups);
            user
        }
        Ok(None) => {
            error!("User info not found in OIDC response");
            audit.deny("the token could not be resolved to a user");
            throttle::record_auth_failure(&data, &proxy, &peer_id).await;
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(e) => {
            error!(error = %e, "couldn't resolve the caller's token");
            audit.deny("the token could not be validated");
            // Only a fault in the caller's own token counts toward a ban; a
            // provider outage must not ban whoever happened to be calling.
            if e.is_caller_fault() {
                throttle::record_auth_failure(&data, &proxy, &peer_id).await;
            }
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };
    let (redirect_front, redirect_kubectl) = super::callback_target(&req);
    let oidc_conf = match proxy
        .get_oidc_conf(data.clone(), redirect_front, redirect_kubectl)
        .await
    {
        Ok(Some(conf)) => conf,
        Ok(None) => {
            error!("OIDC config not found");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(e) => {
            error!(error = %e, "couldn't resolve the OIDC config");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // Resolving the caller is not the same as authorizing them. Every sibling
    // gates on group membership — `kube_redirect` on `is_proxy_allowed`,
    // `get_all_visible_cluster` on `is_user_allowed` — and this endpoint did
    // not, so any authenticated user could start a login against a cluster
    // restricted to a group they are not in. That leaked the cluster's
    // existence and, through the authorize URL below, its issuer, client_id and
    // scopes.
    //
    // 404 rather than 403, matching how the rest of this surface answers: a
    // caller who may not use a cluster must not be able to tell "restricted"
    // from "does not exist", or `proxy_group` stops hiding anything.
    if !proxy.is_proxy_allowed(&user.groups) {
        error!(
            user = %user.username,
            "refusing a login for a cluster the caller is not a member of"
        );
        audit.deny("not a member of the cluster's group");
        return StatusCode::NOT_FOUND.into_response();
    }
    info!(
        "User {:?} is logging in to cluster {:?}",
        user.username, oidc_conf.redirect_url
    );
    let client = match oidc_conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get oidc client");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let scopes: Vec<Scope> = oidc_conf
        .scopes
        .split(' ')
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
    let login_to_callback =
        LoginToCallBackModel::new(nonce.secret().clone(), pkce_verifier.secret().clone());
    if let Err(e) = data
        .redis_set(
            &format!("oidc_csrf_nonce:{}/{}/{}", ns, cluster, csrf_token.secret()),
            &login_to_callback.to_string(),
            Some(CSRF_NONCE_TTL_SECONDS),
        )
        .await
    {
        error!(error = %e, "couldn't store csrf and nonce in redis");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // The documented `text/plain`: without it the generated clients fall back
    // to `application/octet-stream` and refuse the body.
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        auth_url.to_string(),
    )
        .into_response()
}
