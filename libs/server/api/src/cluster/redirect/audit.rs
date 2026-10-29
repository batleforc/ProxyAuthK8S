//! The audit trail (RFC 0008): one JSON line per security decision, in a
//! versioned schema (`audit_schema: 1`).
//!
//! An [`AuditEvent`] is a closed set of fields, built through typed setters:
//! no caller can put a header, a body or a token in it. [`AuditEvent::emit`]
//! writes the line to stdout itself, synchronously, so neither `RUST_LOG` nor
//! `LOG_FORMAT` can silence or reshape it, and a blocked stdout blocks the
//! request rather than losing the trace (decision 7). It also sends a
//! `tracing` event on the `audit` target for the OpenTelemetry span, which
//! the ordinary log layer leaves out.
//!
//! `request_id` and `client_ip` come from the request scope set by the
//! `root_span` middleware ([`scope`]), `trace_id` from the current span: the
//! emitters do not carry them around.

use std::{future::Future, io::Write as _, time::Instant};

use serde_json::{Map, Value, json};

use crate::metrics::{record_audit_event, record_policy_decision};
use crate::model::user::User;

/// Version of the line schema, the `audit_schema` field.
pub const AUDIT_SCHEMA: u64 = 1;

/// Longest `upstream_reason` kept, in characters.
pub const UPSTREAM_REASON_MAX: usize = 256;

/// What an audit line records: the `event` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Request,
    PolicyDecision,
    AuthLogin,
    AuthCallback,
    OauthAuthorize,
    OauthCallback,
    OauthTokenIssued,
    OauthTokenDenied,
    ClientBanned,
    SessionClosed,
    /// RFC 0012: an agent got its certificate.
    AgentEnrolled,
    /// RFC 0012: an enrolment or a tunnel was refused.
    AgentRefused,
    AgentConnected,
    AgentDisconnected,
}

impl Event {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::PolicyDecision => "policy_decision",
            Self::AuthLogin => "auth_login",
            Self::AuthCallback => "auth_callback",
            Self::OauthAuthorize => "oauth_authorize",
            Self::OauthCallback => "oauth_callback",
            Self::OauthTokenIssued => "oauth_token_issued",
            Self::OauthTokenDenied => "oauth_token_denied",
            Self::ClientBanned => "client_banned",
            Self::SessionClosed => "session_closed",
            Self::AgentEnrolled => "agent_enrolled",
            Self::AgentRefused => "agent_refused",
            Self::AgentConnected => "agent_connected",
            Self::AgentDisconnected => "agent_disconnected",
        }
    }
}

/// The `outcome` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    Denied,
    Error,
}

impl Outcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }

    /// A refusal the proxy answered itself: `denied` for a 4xx, `error` for
    /// anything else (a 5xx).
    #[must_use]
    pub fn of_refusal(status: u16) -> Self {
        if (400..500).contains(&status) {
            Self::Denied
        } else {
            Self::Error
        }
    }
}

/// What the request scope knows about the caller before any handler runs.
#[derive(Clone, Debug, Default)]
pub struct RequestMeta {
    /// The `x-request-id` the proxy generated; never one the client sent.
    pub request_id: String,
    /// The client address as bans see it (`TRUSTED_PROXY_COUNT`).
    pub client_ip: String,
}

tokio::task_local! {
    static REQUEST: RequestMeta;
}

/// Run `future` with `meta` as the request scope every audit line reads.
pub async fn scope<F: Future>(meta: RequestMeta, future: F) -> F::Output {
    REQUEST.scope(meta, future).await
}

/// The request scope, when running inside one.
#[must_use]
pub fn current_request() -> Option<RequestMeta> {
    REQUEST.try_with(Clone::clone).ok()
}

#[cfg(feature = "server")]
fn current_trace_id() -> Option<String> {
    trace::helper::get_trace_id()
        .filter(|id| *id != opentelemetry::TraceId::INVALID)
        .map(|id| id.to_string())
}

#[cfg(not(feature = "server"))]
fn current_trace_id() -> Option<String> {
    None
}

/// Strip CR/LF/NUL from a value an identity provider or a client controls.
/// The line is JSON, which escapes them anyway: defence in depth.
fn sanitize_audit_field(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

/// One audit line, built field by field, written by [`AuditEvent::emit`].
#[must_use]
pub struct AuditEvent {
    event: Event,
    outcome: Outcome,
    fields: Map<String, Value>,
}

impl AuditEvent {
    /// The common fields: schema, event, outcome, the request scope's
    /// `request_id` and `client_ip`, the span's `trace_id`, and an unknown
    /// caller (`user: "-"`, no groups).
    pub fn new(event: Event, outcome: Outcome) -> Self {
        let request = current_request().unwrap_or_default();
        let mut fields = Map::new();
        fields.insert("audit_schema".into(), json!(AUDIT_SCHEMA));
        fields.insert("event".into(), json!(event.as_str()));
        fields.insert("outcome".into(), json!(outcome.as_str()));
        fields.insert("request_id".into(), json!(request.request_id));
        fields.insert("trace_id".into(), json!(current_trace_id()));
        fields.insert("client_ip".into(), json!(request.client_ip));
        fields.insert("user".into(), json!("-"));
        fields.insert("groups".into(), json!([]));
        Self {
            event,
            outcome,
            fields,
        }
    }

    fn set(mut self, key: &'static str, value: Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    /// The caller, once known.
    pub fn user(self, user: Option<&str>, groups: &[String]) -> Self {
        let groups: Vec<String> = groups.iter().map(|g| sanitize_audit_field(g)).collect();
        self.set("user", json!(sanitize_audit_field(user.unwrap_or("-"))))
            .set("groups", json!(groups))
    }

    /// The `ProxyKubeApi` the decision is about.
    pub fn cluster(self, ns: &str, cluster: &str) -> Self {
        self.set("ns", json!(ns)).set("cluster", json!(cluster))
    }

    /// A client address computed by the caller, for a line written outside
    /// the request scope.
    pub fn client_ip(self, client_ip: &str) -> Self {
        self.set("client_ip", json!(client_ip))
    }

    /// The proxied request: its HTTP method (and `verb`, its alias until
    /// v1.0.0), the Kubernetes verb when computable, and the path, without
    /// its query string.
    pub fn http(self, method: &str, k8s_verb: Option<&str>, path: &str) -> Self {
        self.set("http_method", json!(method))
            .set("verb", json!(method))
            .set("k8s_verb", json!(k8s_verb))
            .set("path", json!(sanitize_audit_field(path)))
    }

    /// The request a virtual API rewrote this one into.
    pub fn upstream(self, path: &str, k8s_verb: Option<&str>) -> Self {
        self.set("upstream_path", json!(sanitize_audit_field(path)))
            .set("upstream_k8s_verb", json!(k8s_verb))
    }

    /// What an exec or attach asked for (RFC 0004).
    pub fn exec(self, exec: &ExecAudit) -> Self {
        let argv: Vec<String> = exec.argv.iter().map(|e| sanitize_audit_field(e)).collect();
        self.set(
            "exec",
            json!({
                "argv": argv,
                "container": exec.container.as_deref().map(sanitize_audit_field),
                "stdin": exec.stdin,
                "tty": exec.tty,
            }),
        )
    }

    /// The answer's status and how long it took.
    pub fn status(self, status: u16, latency_ms: u64) -> Self {
        self.set("status", json!(status))
            .set("latency_ms", json!(latency_ms))
    }

    /// Why the upstream refused an upgrade, at most [`UPSTREAM_REASON_MAX`]
    /// characters.
    pub fn upstream_reason(self, reason: &str) -> Self {
        let reason: String = sanitize_audit_field(reason)
            .chars()
            .take(UPSTREAM_REASON_MAX)
            .collect();
        self.set("upstream_reason", json!(reason))
    }

    /// A port or verb policy decision.
    pub fn policy(self, policy: &'static str, decision: &'static str, port: Option<u16>) -> Self {
        self.set("policy", json!(policy))
            .set("decision", json!(decision))
            .set("port", json!(port))
    }

    /// Why a login, a callback or a token request was refused. A fixed
    /// phrase from the code, never a value the client sent.
    pub fn reason(self, reason: &'static str) -> Self {
        self.set("reason", json!(reason))
    }

    /// The agent a line is about (RFC 0012): the `epoch` of its certificate,
    /// and the version it announced, at most [`UPSTREAM_REASON_MAX`]
    /// characters.
    pub fn agent(self, epoch: u64, version: Option<&str>) -> Self {
        let version: Option<String> = version.map(|version| {
            sanitize_audit_field(version)
                .chars()
                .take(UPSTREAM_REASON_MAX)
                .collect()
        });
        self.set("agent_epoch", json!(epoch))
            .set("agent_version", json!(version))
    }

    /// The loopback `redirect_uri` of an OAuth flow, without its query.
    pub fn redirect_uri(self, redirect_uri: &str) -> Self {
        let without_query = redirect_uri.split(['?', '#']).next().unwrap_or_default();
        self.set("redirect_uri", json!(sanitize_audit_field(without_query)))
    }

    /// The tokens handed out by `/oauth/token`: never the tokens themselves.
    pub fn grant(self, grant_type: &'static str, expires_in: Option<u64>) -> Self {
        self.set("grant_type", json!(grant_type))
            .set("expires_in", json!(expires_in))
    }

    /// A ban: the failures that triggered it, and its length (`0`: permanent).
    pub fn ban(self, failures: u64, ban_seconds: u64) -> Self {
        self.set("failures", json!(failures))
            .set("ban_seconds", json!(ban_seconds))
    }

    /// The end of a tunnelled session: how long it ran, the bytes each way
    /// (`in`: from the client), and who closed it (`client`, `upstream`,
    /// `filter` or `error`).
    pub fn session(
        self,
        duration_ms: u64,
        bytes_in: u64,
        bytes_out: u64,
        close_reason: &'static str,
    ) -> Self {
        self.set("duration_ms", json!(duration_ms))
            .set("bytes_in", json!(bytes_in))
            .set("bytes_out", json!(bytes_out))
            .set("close_reason", json!(close_reason))
    }

    /// Write the line, count it, and record it on the current span.
    pub fn emit(self) {
        let line = Value::Object(self.fields).to_string();
        write_line(&line);
        record_audit_event(self.event.as_str(), self.outcome.as_str());
        tracing::info!(
            target: "audit",
            event = self.event.as_str(),
            outcome = self.outcome.as_str(),
            "audit event"
        );
    }
}

fn write_line(line: &str) {
    if capture::push(line) {
        return;
    }
    // A write error has nowhere better to go than stdout itself.
    let _write_error = writeln!(std::io::stdout().lock(), "{line}");
}

/// Test support: collect the audit lines of this process instead of writing
/// them to stdout.
#[doc(hidden)]
pub mod capture {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// From now on, keep the lines here rather than on stdout.
    pub fn start() {
        ENABLED.store(true, Ordering::SeqCst);
    }

    pub(super) fn push(line: &str) -> bool {
        if !ENABLED.load(Ordering::SeqCst) {
            return false;
        }
        LINES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(line.to_string());
        true
    }

    /// Every line captured so far, parsed.
    #[must_use]
    pub fn lines() -> Vec<serde_json::Value> {
        LINES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// The audit line of a login, callback or OAuth handler, filled in as the
/// handler learns who the caller is, and written once with its answer by
/// [`HandlerAudit::finish`].
#[derive(Debug, Default)]
pub struct HandlerAudit {
    ns: String,
    cluster: String,
    user: Option<String>,
    groups: Vec<String>,
    reason: Option<&'static str>,
    error: bool,
    redirect_uri: Option<String>,
    grant: Option<(&'static str, Option<u64>)>,
}

impl HandlerAudit {
    #[must_use]
    pub fn new(ns: &str, cluster: &str) -> Self {
        Self {
            ns: ns.to_string(),
            cluster: cluster.to_string(),
            ..Self::default()
        }
    }

    /// The caller, once resolved.
    pub fn user(&mut self, user: &str, groups: &[String]) {
        self.user = Some(user.to_string());
        self.groups = groups.to_vec();
    }

    /// The request is refused, for `reason`.
    pub fn deny(&mut self, reason: &'static str) {
        self.reason = Some(reason);
    }

    /// The request could not be served, for `reason`.
    pub fn fail(&mut self, reason: &'static str) {
        self.reason = Some(reason);
        self.error = true;
    }

    /// The loopback `redirect_uri` of an OAuth flow.
    pub fn redirect_uri(&mut self, redirect_uri: &str) {
        self.redirect_uri = Some(redirect_uri.to_string());
    }

    /// The tokens `/oauth/token` hands out.
    pub fn grant(&mut self, grant_type: &'static str, expires_in: Option<u64>) {
        self.grant = Some((grant_type, expires_in));
    }

    /// Write the line for `response`: `allowed` as `event`, otherwise
    /// `denied_event`. A 5xx, or a redirect carrying the OAuth error
    /// `server_error`, is an `error`; any other 4xx or redirect carrying an
    /// OAuth `error`, or a [`Self::deny`], is `denied`.
    pub fn finish(
        mut self,
        event: Event,
        denied_event: Event,
        response: &axum::response::Response,
    ) {
        let status = response.status();
        if let Some(error) = redirect_error(response) {
            self.reason.get_or_insert(error);
            self.error |= error == "server_error";
        }
        let outcome = if self.error || status.is_server_error() {
            Outcome::Error
        } else if self.reason.is_some() || status.is_client_error() {
            Outcome::Denied
        } else {
            Outcome::Allowed
        };
        let mut line = AuditEvent::new(
            if outcome == Outcome::Allowed {
                event
            } else {
                denied_event
            },
            outcome,
        )
        .user(self.user.as_deref(), &self.groups)
        .cluster(&self.ns, &self.cluster)
        .set("status", json!(status.as_u16()));
        if let Some(reason) = self.reason {
            line = line.reason(reason);
        }
        if let Some(redirect_uri) = &self.redirect_uri {
            line = line.redirect_uri(redirect_uri);
        }
        if let Some((grant_type, expires_in)) = self.grant {
            line = line.grant(grant_type, expires_in);
        }
        line.emit();
    }
}

/// The OAuth `error` a redirect hands the client, as a fixed phrase: the
/// value is the proxy's own (`redirect_with_error`), but it is mapped onto a
/// closed list anyway, so no client-chosen text can reach the audit line.
fn redirect_error(response: &axum::response::Response) -> Option<&'static str> {
    if !response.status().is_redirection() {
        return None;
    }
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)?
        .to_str()
        .ok()?;
    let url = reqwest::Url::parse(location).ok()?;
    let (_, error) = url.query_pairs().find(|(key, _)| key == "error")?;
    Some(match error.as_ref() {
        "access_denied" => "access_denied",
        "invalid_request" => "invalid_request",
        "server_error" => "server_error",
        "unsupported_response_type" => "unsupported_response_type",
        _ => "oauth_error",
    })
}

/// Outcome of the port policy on one request or session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortDecision {
    Allowed,
    Denied,
    NodeProxyDenied,
    ClosedByFilter,
}

impl PortDecision {
    /// The `decision` field of the audit line and the `decision` label of
    /// `proxyauth_policy_decisions_total`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::NodeProxyDenied => "node_proxy_denied",
            Self::ClosedByFilter => "closed_by_filter",
        }
    }
}

/// What an exec or attach request asked for, as the audit records it: the
/// command line already masked when `mask_exec_secrets` is on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecAudit {
    pub argv: Vec<String>,
    pub container: Option<String>,
    pub stdin: bool,
    pub tty: bool,
}

/// Everything known about a proxied request before it is answered.
#[derive(Debug, Clone)]
pub struct AuditContext {
    pub ns: String,
    pub cluster: String,
    /// The HTTP method.
    pub method: String,
    pub path: String,
    pub k8s_verb: Option<String>,
    /// The rewritten request of a virtual API: path and Kubernetes verb.
    pub upstream: Option<(String, Option<String>)>,
    /// What an exec or attach asked for (RFC 0004).
    pub exec: Option<ExecAudit>,
    pub user: Option<String>,
    pub groups: Vec<String>,
    /// `direct`, or `agent` for a cluster reached through its agent tunnel
    /// (RFC 0012, T14).
    pub upstream_via: &'static str,
    /// How the identity is presented upstream (RFC 0014): `Passthrough`,
    /// `FrontProxy` or `Issuer`.
    pub upstream_mode: &'static str,
    /// The identity asserted to the apiserver (RFC 0014 §10.1), outside
    /// `Passthrough`.
    pub upstream_user: Option<String>,
    pub upstream_groups: Vec<String>,
    /// Why the request was refused, when a refusal has a closed reason.
    pub reason: Option<&'static str>,
    started_at: Instant,
}

impl AuditContext {
    #[must_use]
    pub fn new(ns: &str, cluster: &str, method: &str, path: &str) -> Self {
        Self {
            ns: ns.to_string(),
            cluster: cluster.to_string(),
            method: method.to_string(),
            path: path.to_string(),
            k8s_verb: None,
            upstream: None,
            exec: None,
            user: None,
            groups: Vec::new(),
            upstream_via: "direct",
            upstream_mode: "Passthrough",
            upstream_user: None,
            upstream_groups: Vec::new(),
            reason: None,
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

    #[must_use]
    pub fn latency_ms(&self) -> u64 {
        u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// A line with this request's identity: caller, cluster, method, path.
    fn event(&self, event: Event, outcome: Outcome) -> AuditEvent {
        let line = AuditEvent::new(event, outcome)
            .user(self.user.as_deref(), &self.groups)
            .cluster(&self.ns, &self.cluster)
            .http(&self.method, self.k8s_verb.as_deref(), &self.path)
            .set("upstream_via", json!(self.upstream_via))
            .set("upstream_mode", json!(self.upstream_mode));
        let line = match &self.upstream_user {
            Some(user) => line
                .set("upstream_user", json!(user))
                .set("upstream_groups", json!(self.upstream_groups)),
            None => line,
        };
        let line = match self.reason {
            Some(reason) => line.set("reason", json!(reason)),
            None => line,
        };
        let line = match &self.upstream {
            Some((path, verb)) => line.upstream(path, verb.as_deref()),
            None => line,
        };
        match &self.exec {
            Some(exec) => line.exec(exec),
            None => line,
        }
    }

    /// Record a port policy decision: one audit line, and the shared counter.
    /// `port` is `None` when the request named no readable port.
    pub fn emit_port_decision(&self, decision: PortDecision, port: Option<u16>) {
        let outcome = if decision == PortDecision::Allowed {
            Outcome::Allowed
        } else {
            Outcome::Denied
        };
        self.event(Event::PolicyDecision, outcome)
            .policy("port", decision.as_str(), port)
            .emit();
        record_policy_decision(
            &format!("{}/{}", self.ns, self.cluster),
            "port",
            decision.as_str(),
        );
    }

    /// Record an exec policy decision (RFC 0004): one audit line, and the
    /// shared counter. `denied` carries the fixed word of what was refused
    /// (`command`, `container`…), never the command itself.
    pub fn emit_exec_decision(&self, denied: Option<&'static str>) {
        let (outcome, decision) = match denied {
            None => (Outcome::Allowed, "allowed"),
            Some(_) => (Outcome::Denied, "denied"),
        };
        let line = self
            .event(Event::PolicyDecision, outcome)
            .policy("exec", decision, None);
        match denied {
            Some(reason) => line.reason(reason),
            None => line,
        }
        .emit();
        record_policy_decision(&format!("{}/{}", self.ns, self.cluster), "exec", decision);
    }

    /// Record a request refused for its Kubernetes verb: one audit line, and
    /// the shared counter.
    pub fn emit_verb_denied(&self) {
        self.event(Event::PolicyDecision, Outcome::Denied)
            .policy("verb", "denied", None)
            .emit();
        record_policy_decision(&format!("{}/{}", self.ns, self.cluster), "verb", "denied");
    }

    /// The `request` line of an answer the upstream gave: the proxy let the
    /// request through, whatever the status.
    pub fn emit(&self, status: u16) {
        self.event(Event::Request, Outcome::Allowed)
            .status(status, self.latency_ms())
            .emit();
    }

    /// The `request` line of an upgrade the upstream refused, with its reason
    /// when the upstream gave one.
    pub fn emit_upstream_refusal(&self, status: u16, reason: Option<&str>) {
        let line = self
            .event(Event::Request, Outcome::Allowed)
            .status(status, self.latency_ms());
        match reason {
            Some(reason) => line.upstream_reason(reason),
            None => line,
        }
        .emit();
    }

    /// The `session_closed` line of a tunnel: `error` when it ended on a
    /// failure, `allowed` otherwise (the filter's refusal has its own
    /// `policy_decision` line).
    /// `channel_bytes` and `channel` are an exec's under restriction
    /// (RFC 0004): the bytes of each channel, and the forbidden channel that
    /// ended it.
    pub fn emit_session_closed(
        &self,
        duration_ms: u64,
        bytes_in: u64,
        bytes_out: u64,
        close_reason: &'static str,
        channel_bytes: Option<&std::collections::BTreeMap<u8, u64>>,
        channel: Option<u8>,
    ) {
        let outcome = if close_reason == "error" {
            Outcome::Error
        } else {
            Outcome::Allowed
        };
        let mut line = self.event(Event::SessionClosed, outcome).session(
            duration_ms,
            bytes_in,
            bytes_out,
            close_reason,
        );
        if let Some(bytes) = channel_bytes {
            let bytes: Map<String, Value> = bytes
                .iter()
                .map(|(channel, count)| (channel.to_string(), json!(count)))
                .collect();
            line = line.set("channel_bytes", Value::Object(bytes));
        }
        if let Some(channel) = channel {
            line = line.set("channel", json!(channel));
        }
        line.emit();
    }

    /// The `request` line of a request the proxy refused itself (`denied`) or
    /// could not serve (`error`).
    pub fn emit_refusal(&self, status: u16) {
        self.event(Event::Request, Outcome::of_refusal(status))
            .status(status, self.latency_ms())
            .emit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(event: AuditEvent) -> Value {
        Value::Object(event.fields)
    }

    #[test]
    fn a_request_line_carries_the_common_and_http_fields() {
        let mut ctx = AuditContext::new("dev", "c", "GET", "/api/v1/namespaces/dev/pods");
        ctx.k8s_verb = Some("list".into());
        ctx.user = Some("alice".into());
        ctx.groups = vec!["dev".into()];
        let event = line(ctx.event(Event::Request, Outcome::Allowed).status(200, 12));
        assert_eq!(event["audit_schema"], 1);
        assert_eq!(event["event"], "request");
        assert_eq!(event["outcome"], "allowed");
        assert_eq!(event["user"], "alice");
        assert_eq!(event["groups"], json!(["dev"]));
        assert_eq!(event["ns"], "dev");
        assert_eq!(event["cluster"], "c");
        assert_eq!(event["http_method"], "GET");
        assert_eq!(event["k8s_verb"], "list");
        assert_eq!(event["path"], "/api/v1/namespaces/dev/pods");
        assert_eq!(event["status"], 200);
        assert_eq!(event["latency_ms"], 12);
        for key in ["request_id", "trace_id", "client_ip"] {
            assert!(event.get(key).is_some(), "{key} missing");
        }
    }

    #[test]
    fn verb_stays_an_alias_of_the_http_method() {
        let event = line(AuditEvent::new(Event::Request, Outcome::Allowed).http(
            "POST",
            Some("create"),
            "/x",
        ));
        assert_eq!(event["verb"], "POST");
        assert_eq!(event["verb"], event["http_method"]);
    }

    #[test]
    fn an_unknown_caller_is_a_dash_with_no_groups() {
        let event = line(AuditEvent::new(Event::AuthLogin, Outcome::Denied));
        assert_eq!(event["user"], "-");
        assert_eq!(event["groups"], json!([]));
    }

    #[test]
    fn a_forged_user_stays_one_field() {
        // T4: a value carrying a newline and a fake line cannot split the line.
        let forged = "alice\n{\"audit_schema\":1,\"event\":\"request\"}";
        let event = AuditEvent::new(Event::Request, Outcome::Allowed).user(Some(forged), &[]);
        let text = Value::Object(event.fields).to_string();
        assert_eq!(text.lines().count(), 1);
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert!(parsed["user"].as_str().unwrap().starts_with("alice"));
    }

    #[test]
    fn the_upstream_reason_is_capped() {
        let event = line(
            AuditEvent::new(Event::Request, Outcome::Allowed).upstream_reason(&"x".repeat(1000)),
        );
        assert_eq!(
            event["upstream_reason"].as_str().unwrap().chars().count(),
            UPSTREAM_REASON_MAX
        );
    }

    #[test]
    fn the_redirect_uri_loses_its_query() {
        let event = line(
            AuditEvent::new(Event::OauthAuthorize, Outcome::Allowed)
                .redirect_uri("http://127.0.0.1:8000/cb?code=secret#x"),
        );
        assert_eq!(event["redirect_uri"], "http://127.0.0.1:8000/cb");
    }

    #[test]
    fn refusals_are_denied_or_error_by_status() {
        assert_eq!(Outcome::of_refusal(403), Outcome::Denied);
        assert_eq!(Outcome::of_refusal(429), Outcome::Denied);
        assert_eq!(Outcome::of_refusal(503), Outcome::Error);
    }

    #[tokio::test]
    async fn the_request_scope_fills_request_id_and_client_ip() {
        let meta = RequestMeta {
            request_id: "id-1".into(),
            client_ip: "10.0.0.7".into(),
        };
        let event = scope(meta, async {
            line(AuditEvent::new(Event::Request, Outcome::Allowed))
        })
        .await;
        assert_eq!(event["request_id"], "id-1");
        assert_eq!(event["client_ip"], "10.0.0.7");
    }

    #[test]
    fn a_virtual_api_line_names_both_requests() {
        let mut ctx =
            AuditContext::new("dev", "c", "GET", "/apis/project.openshift.io/v1/projects");
        ctx.k8s_verb = Some("list".into());
        ctx.upstream = Some(("/api/v1/namespaces".into(), Some("list".into())));
        let event = line(ctx.event(Event::Request, Outcome::Allowed));
        assert_eq!(event["upstream_path"], "/api/v1/namespaces");
        assert_eq!(event["upstream_k8s_verb"], "list");
    }
}
