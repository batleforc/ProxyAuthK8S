//! Mediated OAuth 2.0 Authorization Server (RFC 6749 + PKCE), per cluster.
//!
//! Advertised by [`crate::cluster::auth::well_known`]. The external caller
//! never needs a client registered with the cluster's upstream OIDC provider:
//! this proxy is that registered client, and mediates the whole exchange —
//! `/oauth/authorize` starts its own PKCE round-trip with the upstream
//! provider, `/oauth/callback` receives the upstream redirect and mints a
//! proxy-owned one-time code, and `/oauth/token` exchanges that code for the
//! upstream tokens once the external caller proves possession of its own PKCE
//! verifier.

pub mod authorize;
pub mod callback;
pub mod jwks;
pub mod model;
pub mod token;

use reqwest::Url;

/// Redirect to `redirect_uri` with an RFC 6749 §4.1.2.1 error response.
///
/// Used once `redirect_uri` itself has been validated: from that point on,
/// failures are reported to the external client via redirect rather than a
/// bare error body, so a CLI driving the flow can surface them.
pub(crate) fn redirect_with_error(redirect_uri: &Url, error: &str, state: Option<&str>) -> actix_web::HttpResponse {
    let mut url = redirect_uri.clone();
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("error", error);
        if let Some(state) = state {
            pairs.append_pair("state", state);
        }
    }
    actix_web::HttpResponse::Found()
        .insert_header(("Location", url.to_string()))
        .finish()
}
