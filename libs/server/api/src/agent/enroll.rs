//! `POST /agent/v1/enroll` (RFC 0012 §5.2, `ServiceAccountToken`).
//!
//! The agent sends a CSR and a projected ServiceAccount token of its own
//! cluster; the token is checked against that cluster's public keys, as the
//! `ProxyKubeApi` names them. A valid proof gets a 24 h certificate for the
//! CR's identity (namespace, name, UID, `epoch`), never for one the request
//! names. Every refusal is a bare `401`: the reason goes to the logs only.

use std::sync::Arc;

use agent_pki::AgentIdentity;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crd::ProxyKubeApi;
use crd::service::{
    AgentBootstrap, JwksSource, MAX_JWKS_BYTES, Service, ServiceAccountTokenBootstrap,
};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use kube::ResourceExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::listener::{Gateway, Reason};
use crate::cluster::redirect::audit::{AuditEvent, Event, Outcome};
use crate::cluster::redirect::forwarded::throttle_client_ip;
use crate::helper::{forwarded_for, peer_ip};
use crate::metrics::record_agent_enrollment;

/// Bound on an enrolment body: a CSR and a token.
pub const MAX_ENROLL_BYTES: usize = 16 * 1024;
const WINDOW_SECONDS: i64 = 60;

#[derive(Deserialize)]
pub struct EnrollRequest {
    /// `<namespace>/<name>` of the `ProxyKubeApi`.
    pub cluster: String,
    /// PEM.
    pub csr: String,
    proof: Proof,
}

/// The enrolment body, `None` when it is not one (public for the
/// `fuzz_agent_enroll` target).
#[must_use]
pub fn parse_body(body: &[u8]) -> Option<EnrollRequest> {
    serde_json::from_slice(body).ok()
}

const METHOD: &str = "service_account_token";

/// The `agent_refused` line and count of a refused enrolment.
fn refused(client: &str, cluster: Option<&str>, reason: Reason) {
    warn!(%client, cluster, reason, "agent enrolment refused");
    record_agent_enrollment(METHOD, "refused");
    let line = AuditEvent::new(Event::AgentRefused, Outcome::Denied)
        .client_ip(client)
        .reason(reason);
    match cluster.and_then(|cluster| cluster.split_once('/')) {
        Some((ns, name)) => line.cluster(ns, name),
        None => line,
    }
    .emit();
}

#[derive(Deserialize)]
struct Proof {
    service_account_token: String,
}

#[derive(Serialize)]
struct EnrollResponse {
    /// The agent certificate, PEM.
    cert: String,
    /// The agent CA, PEM: the trust anchor of the tunnel TLS.
    ca: String,
}

pub(super) async fn enroll(State(gateway): State<Arc<Gateway>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let client = throttle_client_ip(forwarded_for(&parts.headers), peer_ip(&parts));
    if let Err(reason) = gateway.within_limits(&client).await {
        refused(&client, None, reason);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Ok(body) = axum::body::to_bytes(body, MAX_ENROLL_BYTES).await else {
        refused(&client, None, "body too large");
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Some(request) = parse_body(&body) else {
        refused(&client, None, "body");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let issued = async {
        let identity = gateway.verify(&request).await?;
        let cert = gateway
            .ca
            .issue_agent(&request.csr, &identity)
            .map_err(|_| "csr")?;
        Ok::<_, Reason>((identity, cert))
    };
    match issued.await {
        Ok((identity, cert)) => {
            info!(cluster = %identity.cluster(), epoch = identity.epoch, %client, "agent enrolled");
            record_agent_enrollment(METHOD, "enrolled");
            AuditEvent::new(Event::AgentEnrolled, Outcome::Allowed)
                .client_ip(&client)
                .cluster(&identity.namespace, &identity.name)
                .agent(identity.epoch, None)
                .emit();
            Json(EnrollResponse {
                cert,
                ca: gateway.ca.cert_pem().to_string(),
            })
            .into_response()
        }
        Err(reason) => {
            refused(&client, Some(&request.cluster), reason);
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
}

impl Gateway {
    /// Per client address and in total, per minute (§7.5). Fails closed:
    /// without Redis, no enrolment.
    async fn within_limits(&self, client: &str) -> Result<(), Reason> {
        let limits = [
            (
                format!("{}:agent-enroll:{client}", crd::REDIS_PREFIX),
                self.settings.enroll_per_minute,
            ),
            (
                format!("{}:agent-enroll", crd::REDIS_PREFIX),
                self.settings.enroll_per_minute_total,
            ),
        ];
        for (key, limit) in limits {
            let used = self
                .state
                .incr_with_ttl(&key, WINDOW_SECONDS)
                .await
                .map_err(|_| "redis")?;
            if usize::try_from(used).unwrap_or(usize::MAX) > limit {
                return Err("rate limit");
            }
        }
        Ok(())
    }

    async fn verify(&self, request: &EnrollRequest) -> Result<AgentIdentity, Reason> {
        let proxy: ProxyKubeApi = self
            .state
            .get_object_from_redis(crd::REDIS_PREFIX, &request.cluster)
            .await
            .map_err(|_| "redis")?
            .ok_or("unknown cluster")?;
        let Service::Agent(agent) = &proxy.spec.service else {
            return Err("not an agent cluster");
        };
        let AgentBootstrap::ServiceAccountToken(bootstrap) = &agent.bootstrap;
        let token = &request.proof.service_account_token;
        let kid = jsonwebtoken::decode_header(token).map_err(|_| "token")?.kid;
        let namespace = proxy.namespace().unwrap_or_default();
        let jwk = self.jwk(bootstrap, &namespace, kid.as_deref()).await?;
        let claims = jwt_validator::verify_with_key(token, &jwk, &bootstrap.audience)
            .map_err(|_| "token")?;
        if claims.get("sub").and_then(serde_json::Value::as_str) != Some(bootstrap.subject.as_str())
        {
            return Err("subject");
        }
        Ok(AgentIdentity {
            namespace,
            name: proxy.name_any(),
            uid: proxy.metadata.uid.clone().ok_or("uid")?,
            epoch: agent.epoch,
        })
    }

    /// The key `kid` names in the cluster's JWKS; without a `kid`, the only
    /// key of a one-key set (`jwt_validator::jwks::select_key`).
    async fn jwk(
        &self,
        bootstrap: &ServiceAccountTokenBootstrap,
        namespace: &str,
        kid: Option<&str>,
    ) -> Result<Jwk, Reason> {
        if let Some(url) = &bootstrap.jwks_url {
            // T13, for a CR that skipped admission.
            if !url.starts_with("https://") {
                return Err("jwks");
            }
            let http = self.http.as_ref().ok_or("jwks")?;
            return self.jwks.key_at(http, url, kid).await.map_err(|_| "jwks");
        }
        let text = match (&bootstrap.jwks, &bootstrap.jwks_from) {
            (Some(inline), _) => inline.clone(),
            (None, Some(source)) => read_jwks_from(&self.state, source, namespace).await?,
            (None, None) => return Err("jwks"),
        };
        if text.len() > MAX_JWKS_BYTES {
            return Err("jwks");
        }
        let set: JwkSet = serde_json::from_str(&text).map_err(|_| "jwks")?;
        jwt_validator::jwks::select_key(&set, kid)
            .cloned()
            .ok_or("unknown key")
    }
}

// shortcut: read at every enrolment, no 30 s cache (§5.2); enrolments are
// rate-limited to `enroll_per_minute_total` reads a minute at most.
async fn read_jwks_from(
    state: &common::State,
    source: &JwksSource,
    namespace: &str,
) -> Result<String, Reason> {
    source
        .as_cert_source()
        .get_cert(state.client.clone(), namespace)
        .await
        .map_err(|_| "jwks")?
        .ok_or("jwks")
}
