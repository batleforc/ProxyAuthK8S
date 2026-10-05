use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use common::State;
use crd_runtime::ProxyKubeApiRuntime;
use openidconnect::{
    AccessTokenHash, AuthorizationCode, Nonce, OAuth2TokenResponse, PkceCodeVerifier, TokenResponse,
};
use reqwest::Url;
use serde::Deserialize;
use tracing::{error, info, instrument};
use utoipa::{IntoParams, ToSchema};

use crate::cluster::auth::{
    load_discovery_enabled_proxy,
    oauth::{
        model::{CODE_PREFIX, CODE_TTL_SECONDS, IssuedCode, PENDING_PREFIX, PendingAuthorization},
        redirect_with_error,
    },
    throttle_oauth_as,
};
use crate::helper::extract_ns_cluster;

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct OAuthCallbackQuery {
    /// Authorization code from the upstream OIDC provider.
    pub code: String,
    /// The correlation id this proxy generated at `/oauth/authorize`.
    pub state: String,
}

/// Callback from the cluster's upstream OIDC provider, for the mediated OAuth Authorization Server flow
///
/// Not meant to be opened directly: the upstream provider redirects here after
/// the caller authenticates. On success, redirects to the external client's
/// own `redirect_uri` with a proxy-minted authorization code.
#[utoipa::path(
    tag = "auth_clusters",
    responses(
        (status = 302, description = "Redirect to the external client's redirect_uri, with a proxy-minted code (or an error)."),
        (status = 400, description = "Unknown or expired state."),
        (status = 404, description = "Cluster not found, disabled, or discovery not enabled."),
        (status = 500, description = "Internal server error."),
    ),
    params(
        ("ns" = String, description = "Namespace containing the cluster."),
        ("cluster" = String, description = "Cluster name that should exist in the namespace."),
        OAuthCallbackQuery,
    )
)]
#[get("/{ns}/{cluster}/oauth/callback")]
#[instrument(name = "oauth_as_callback", skip(data, callback))]
pub async fn callback(
    req: HttpRequest,
    data: web::Data<State>,
    callback: web::Query<OAuthCallbackQuery>,
) -> impl Responder {
    let (ns, cluster) = if let Some(parts) = extract_ns_cluster(&req) {
        parts
    } else {
        error!(path = %req.path(), "missing ns/cluster path parameters");
        return HttpResponse::NotFound().finish();
    };
    let proxy = match load_discovery_enabled_proxy(&data, &ns, &cluster).await {
        Ok(proxy) => proxy,
        Err(gate) => return gate.into_response(),
    };
    if let Some(response) = throttle_oauth_as(&req, &data, &proxy).await {
        return response;
    }

    let pending_key = format!("{PENDING_PREFIX}:{ns}/{cluster}/{}", callback.state);
    // Single-use: read and delete atomically so the same upstream `code`+`state`
    // cannot be replayed, even by concurrent requests.
    let pending = match data.redis_take(&pending_key).await {
        Ok(Some(raw)) => match serde_json::from_str::<PendingAuthorization>(&raw) {
            Ok(pending) => pending,
            Err(e) => {
                error!(error = %e, "couldn't parse pending authorization");
                return HttpResponse::BadRequest().body("invalid state");
            }
        },
        Ok(None) => {
            error!("pending authorization not found");
            return HttpResponse::BadRequest().body("invalid or expired state");
        }
        Err(e) => {
            error!(error = %e, "couldn't get pending authorization");
            return HttpResponse::ServiceUnavailable().finish();
        }
    };
    // Already validated (loopback-only) when stored; re-parsing here just
    // recovers the `Url` needed to build the redirect / error responses.
    let Ok(client_redirect_uri) = Url::parse(&pending.client_redirect_uri) else {
        error!("stored client_redirect_uri is not a valid URL");
        return HttpResponse::InternalServerError().finish();
    };

    let oauth_conf = if let Some(conf) = proxy.get_oauth_as_oidc_conf(data.clone().into_inner()) {
        conf
    } else {
        error!("OIDC config not found");
        return redirect_with_error(
            &client_redirect_uri,
            "server_error",
            pending.client_state.as_deref(),
        );
    };
    let client_reqwest = match oauth_conf.oidc_reqwest_client() {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't build oidc http client");
            return redirect_with_error(
                &client_redirect_uri,
                "server_error",
                pending.client_state.as_deref(),
            );
        }
    };
    let client_oidc = match oauth_conf.oidc_core().await {
        Ok(client) => client,
        Err(e) => {
            error!(error = %e, "couldn't get oidc client");
            return redirect_with_error(
                &client_redirect_uri,
                "server_error",
                pending.client_state.as_deref(),
            );
        }
    };
    info!(%ns, %cluster, "OAuth AS callback received for cluster");

    let exchange_code =
        match client_oidc.exchange_code(AuthorizationCode::new(callback.code.clone())) {
            Ok(code) => code,
            Err(e) => {
                error!(error = %e, "couldn't exchange code");
                return redirect_with_error(
                    &client_redirect_uri,
                    "server_error",
                    pending.client_state.as_deref(),
                );
            }
        };
    let token_response = match exchange_code
        .set_pkce_verifier(PkceCodeVerifier::new(pending.upstream_pkce_verifier))
        .request_async(&client_reqwest)
        .await
    {
        Ok(token) => token,
        Err(e) => {
            error!(error = %e, "couldn't get token response");
            return redirect_with_error(
                &client_redirect_uri,
                "access_denied",
                pending.client_state.as_deref(),
            );
        }
    };

    let id_token = if let Some(id_token) = token_response.id_token() {
        id_token
    } else {
        error!("No ID token received");
        return redirect_with_error(
            &client_redirect_uri,
            "server_error",
            pending.client_state.as_deref(),
        );
    };
    let id_token_verifier = client_oidc.id_token_verifier();
    let claims = match id_token.claims(&id_token_verifier, &Nonce::new(pending.upstream_nonce)) {
        Ok(claims) => claims,
        Err(e) => {
            error!(error = %e, "couldn't verify ID token");
            return redirect_with_error(
                &client_redirect_uri,
                "server_error",
                pending.client_state.as_deref(),
            );
        }
    };
    if let Some(expected_access_token_hash) = claims.access_token_hash() {
        let signing_alg = match id_token.signing_alg() {
            Ok(alg) => alg,
            Err(e) => {
                error!(error = %e, "couldn't get signing alg");
                return redirect_with_error(
                    &client_redirect_uri,
                    "server_error",
                    pending.client_state.as_deref(),
                );
            }
        };
        let signing_key = match id_token.signing_key(&id_token_verifier) {
            Ok(key) => key,
            Err(e) => {
                error!(error = %e, "couldn't get signing key");
                return redirect_with_error(
                    &client_redirect_uri,
                    "server_error",
                    pending.client_state.as_deref(),
                );
            }
        };
        let actual_access_token_hash = match AccessTokenHash::from_token(
            token_response.access_token(),
            signing_alg,
            signing_key,
        ) {
            Ok(hash) => hash,
            Err(e) => {
                error!(error = %e, "couldn't get access token hash");
                return redirect_with_error(
                    &client_redirect_uri,
                    "server_error",
                    pending.client_state.as_deref(),
                );
            }
        };
        if actual_access_token_hash != *expected_access_token_hash {
            return redirect_with_error(
                &client_redirect_uri,
                "access_denied",
                pending.client_state.as_deref(),
            );
        }
    }

    let issued_code_value = openidconnect::CsrfToken::new_random().secret().clone();
    let issued = IssuedCode {
        redirect_uri: pending.client_redirect_uri.clone(),
        code_challenge: pending.client_code_challenge,
        access_token: token_response.access_token().secret().clone(),
        refresh_token: token_response
            .refresh_token()
            .map(|token| token.secret().clone()),
        id_token: id_token.to_string(),
        scope: oauth_conf.scopes.clone(),
        expires_in: token_response
            .expires_in()
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)),
    };
    let issued_json = match serde_json::to_string(&issued) {
        Ok(json) => json,
        Err(e) => {
            error!(error = %e, "couldn't serialize issued code");
            return redirect_with_error(
                &client_redirect_uri,
                "server_error",
                pending.client_state.as_deref(),
            );
        }
    };
    if let Err(e) = data
        .redis_set(
            &format!("{CODE_PREFIX}:{ns}/{cluster}/{issued_code_value}"),
            &issued_json,
            Some(CODE_TTL_SECONDS),
        )
        .await
    {
        error!(error = %e, "couldn't store issued code");
        return redirect_with_error(
            &client_redirect_uri,
            "server_error",
            pending.client_state.as_deref(),
        );
    }

    info!(subject = %claims.subject().as_str(), "OAuth AS mediated login succeeded");
    let mut redirect = client_redirect_uri;
    {
        let mut pairs = redirect.query_pairs_mut();
        pairs.append_pair("code", &issued_code_value);
        if let Some(state) = &pending.client_state {
            pairs.append_pair("state", state);
        }
    }
    HttpResponse::Found()
        .insert_header(("Location", redirect.to_string()))
        .finish()
}
