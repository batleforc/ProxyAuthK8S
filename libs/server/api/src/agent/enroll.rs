//! `POST /agent/v1/enroll` (RFC 0012 §5.2).
//!
//! The agent sends a CSR and a proof: a projected ServiceAccount token of its
//! own cluster, checked against that cluster's public keys as the
//! `ProxyKubeApi` names them, or the `OneShotToken` the controller wrote,
//! taken from Redis by its hash (`GETDEL`, T6). A valid proof gets a 24 h
//! certificate for the
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

/// The `agent_refused` line and count of a refused enrolment.
fn refused(client: &str, cluster: Option<&str>, method: &'static str, reason: Reason) {
    warn!(%client, cluster, reason, "agent enrolment refused");
    record_agent_enrollment(method, "refused");
    let line = AuditEvent::new(Event::AgentRefused, Outcome::Denied)
        .client_ip(client)
        .reason(reason);
    match cluster.and_then(|cluster| cluster.split_once('/')) {
        Some((ns, name)) => line.cluster(ns, name),
        None => line,
    }
    .emit();
}

/// One of the two, as the CR's `bootstrap` asks.
#[derive(Deserialize)]
struct Proof {
    service_account_token: Option<String>,
    one_shot_token: Option<String>,
}

impl Proof {
    /// The `method` label of `proxyauth_agent_enrollments_total`.
    fn method(&self) -> &'static str {
        if self.one_shot_token.is_some() {
            "one_shot_token"
        } else {
            "service_account_token"
        }
    }
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
    // Before the body is read: the method is not known yet.
    let unknown = "unknown";
    if let Err(reason) = gateway.within_limits(&client).await {
        refused(&client, None, unknown, reason);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Ok(body) = axum::body::to_bytes(body, MAX_ENROLL_BYTES).await else {
        refused(&client, None, unknown, "body too large");
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Some(request) = parse_body(&body) else {
        refused(&client, None, unknown, "body");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let method = request.proof.method();
    let issued = async {
        let (identity, one_shot) = gateway.verify(&request).await?;
        let cert = gateway
            .ca
            .issue_agent(&request.csr, &identity)
            .map_err(|_| "csr")?;
        // Taken last: a request refused for its CSR burns no token.
        if let Some(key) = one_shot {
            gateway
                .state
                .redis_take(&key)
                .await
                .map_err(|_| "redis")?
                .ok_or("token")?;
        }
        Ok::<_, Reason>((identity, cert))
    };
    match issued.await {
        Ok((identity, cert)) => {
            info!(cluster = %identity.cluster(), epoch = identity.epoch, %client, "agent enrolled");
            record_agent_enrollment(method, "enrolled");
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
            refused(&client, Some(&request.cluster), method, reason);
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

    /// The identity a valid proof enrols, and for a `OneShotToken` the Redis
    /// key to take before the certificate goes out.
    async fn verify(
        &self,
        request: &EnrollRequest,
    ) -> Result<(AgentIdentity, Option<String>), Reason> {
        let proxy: ProxyKubeApi = self
            .state
            .get_proxy_from_redis(&request.cluster)
            .await
            .map_err(|_| "redis")?
            .ok_or("unknown cluster")?;
        let Service::Agent(agent) = &proxy.spec.service else {
            return Err("not an agent cluster");
        };
        let namespace = proxy.namespace().unwrap_or_default();
        let identity = AgentIdentity {
            namespace: namespace.clone(),
            name: proxy.name_any(),
            uid: proxy.metadata.uid.clone().ok_or("uid")?,
            epoch: agent.epoch,
        };
        let bootstrap = match &agent.bootstrap {
            AgentBootstrap::ServiceAccountToken(bootstrap) => bootstrap,
            // No enrolment: the certificate comes from the CR's own issuer.
            AgentBootstrap::ExternalCA(_) => return Err("method"),
            AgentBootstrap::OneShotToken(_) => {
                let token = request.proof.one_shot_token.as_deref().ok_or("method")?;
                let key = common::agent_gateway::bootstrap_key(&identity.uid, token);
                return Ok((identity, Some(key)));
            }
        };
        let token = request
            .proof
            .service_account_token
            .as_deref()
            .ok_or("method")?;
        let kid = jsonwebtoken::decode_header(token).map_err(|_| "token")?.kid;
        let jwk = self.jwk(bootstrap, &namespace, kid.as_deref()).await?;
        let claims = jwt_validator::verify_with_key(token, &jwk, &bootstrap.audience)
            .map_err(|_| "token")?;
        if claims.get("sub").and_then(serde_json::Value::as_str) != Some(bootstrap.subject.as_str())
        {
            return Err("subject");
        }
        Ok((identity, None))
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
