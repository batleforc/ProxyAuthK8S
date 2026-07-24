//! Audit trail for proxied requests.
//!
//! One structured `tracing` event per request, on the dedicated `audit` target
//! so it can be routed independently while still flowing through the existing
//! OpenTelemetry pipeline unchanged.

use std::time::Instant;

use crate::model::user::User;

/// Strip CR/LF/NUL so a value carried from the OIDC provider (username, groups)
/// or the request path cannot forge extra lines under a plain-text log sink.
fn sanitize_audit_field(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// Everything known about a proxied request before it is answered.
#[derive(Debug, Clone)]
pub struct AuditContext {
    pub ns: String,
    pub cluster: String,
    pub verb: String,
    pub path: String,
    pub user: Option<String>,
    pub groups: Vec<String>,
    started_at: Instant,
}

impl AuditContext {
    pub fn new(ns: &str, cluster: &str, verb: &str, path: &str) -> Self {
        Self {
            ns: ns.to_string(),
            cluster: cluster.to_string(),
            verb: verb.to_string(),
            path: path.to_string(),
            user: None,
            groups: Vec::new(),
            started_at: Instant::now(),
        }
    }

    /// Attach the caller once authentication resolved one.
    pub fn with_user(&mut self, user: Option<&User>) -> &mut Self {
        if let Some(user) = user {
            self.user = Some(user.username.clone());
            self.groups = user.groups.clone();
        }
        self
    }

    pub fn latency_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Emit the audit event for a request that has been answered with `status`.
    pub fn emit(&self, status: u16) {
        tracing::info!(
            target: "audit",
            user = %sanitize_audit_field(self.user.as_deref().unwrap_or("-")),
            groups = %sanitize_audit_field(&self.groups.join(",")),
            ns = %self.ns,
            cluster = %self.cluster,
            verb = %self.verb,
            path = %sanitize_audit_field(&self.path),
            response_status = status,
            latency_ms = self.latency_ms(),
            "proxied request"
        );
    }
}
