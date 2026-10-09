use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use common::State;
use crd::ProxyKubeApi;
use crd_runtime::ProxyKubeApiRuntime;
use openidconnect::{CsrfToken, Nonce, PkceCodeChallenge, Scope, core::CoreAuthenticationFlow};
use tracing::{error, info, instrument};

use crate::{
    cluster::auth::{auth_model::LoginToCallBackModel, throttle_oauth_as},
    cluster::redirect::{forwarded, throttle},
    helper::{extract_authorization_header, extract_ns_cluster},
    model::user::User,
};

/// Redirect to the cluster's login page
///
/// If the cluster is not found or disabled, return 404.
#[utoipa::path(
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
#[get("/{ns}/{cluster}/auth/login")]
#[instrument(name = "cluster_login", skip(data))]
pub async fn cluster_login(req: HttpRequest, data: web::Data<State>) -> impl Responder {
    /// The CSRF token and nonce only need to survive the redirect round-trip.
    const CSRF_NONCE_TTL_SECONDS: u64 = 300;

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
    if !proxy.spec.enabled
        || proxy
            .spec
            .auth_config
            .as_ref()
            .is_some_and(|auth_config| !auth_config.oidc_provider.enabled)
    {
        return HttpResponse::NotFound().finish();
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
    let peer_ip = req.peer_addr().map(|addr| addr.ip());
    let forwarded_for = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok());
    let peer_id = forwarded::throttle_client_ip(forwarded_for, peer_ip);

    let token = match extract_authorization_header(&req) {
        Ok(token) => token,
        Err(e) => {
            error!("Authorization header extraction failed: {}", e);
            throttle::record_auth_failure(&data, &proxy, &peer_id).await;
            return HttpResponse::Unauthorized().finish();
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
            user
        }
        Ok(None) => {
            error!("User info not found in OIDC response");
            throttle::record_auth_failure(&data, &proxy, &peer_id).await;
            return HttpResponse::Unauthorized().finish();
        }
        Err(e) => {
            error!(error = %e, "couldn't resolve the caller's token");
            // Only a fault in the caller's own token counts toward a ban; a
            // provider outage must not ban whoever happened to be calling.
            if e.is_caller_fault() {
                throttle::record_auth_failure(&data, &proxy, &peer_id).await;
            }
            return HttpResponse::Unauthorized().finish();
        }
    };
    let redirect_front = req.headers().contains_key("x-front-callback");
    let redirect_kubectl = req
        .headers()
        .get("x-kubectl-callback")
        .map(|v| v.to_str().unwrap_or_default().to_string());
    let oidc_conf = match proxy
        .get_oidc_conf(data.clone().into_inner(), redirect_front, redirect_kubectl)
        .await
    {
        Ok(Some(conf)) => conf,
        Ok(None) => {
            error!("OIDC config not found");
            return HttpResponse::InternalServerError().finish();
        }
        Err(e) => {
            error!(error = %e, "couldn't resolve the OIDC config");
            return HttpResponse::InternalServerError().finish();
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
        return HttpResponse::NotFound().finish();
    }
    info!(
        "User {:?} is logging in to cluster {:?}",
        user.username, oidc_conf.redirect_url
    );
    let client = match oidc_conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get oidc client");
            return HttpResponse::InternalServerError().finish();
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
        return HttpResponse::InternalServerError().finish();
    }

    HttpResponse::Ok().body(auth_url.to_string())
}
