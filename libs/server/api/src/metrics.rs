//! Prometheus pull endpoint (`GET /management/metrics`) and the HTTP server
//! request-duration instrument it (and the OTLP export) reports.
//!
//! The metrics are recorded through OpenTelemetry; when
//! `METRICS_PROMETHEUS_ENABLED` is set, the `trace` crate also attaches a
//! Prometheus reader to the meter provider and hands its registry to the app
//! as [`PrometheusRegistry`], a request extension.

use std::{sync::LazyLock, time::Instant};

use axum::{
    extract::{MatchedPath, Request, State},
    http::{StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter},
};
use prometheus::{Encoder, TextEncoder};
use tracing::{instrument, warn};

/// `Content-Type` of the Prometheus text exposition format 0.0.4.
pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Name of the HTTP server request-duration histogram (OpenTelemetry semantic
/// conventions; exposed to Prometheus as `http_server_request_duration_seconds`).
pub const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";

/// Name of the counter of access-policy decisions (exposed to Prometheus as
/// `proxyauth_policy_decisions_total`), shared by the port, verb and exec
/// policies. Labels: `cluster` (`{ns}/{name}`), `policy`, `decision`; never a
/// port or a user, to keep the cardinality bounded.
pub const POLICY_DECISIONS: &str = "proxyauth.policy.decisions";

/// Created on first use, once the server has installed the meter provider.
static POLICY_DECISIONS_COUNTER: LazyLock<Counter<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter("proxyauthk8s")
        .u64_counter(POLICY_DECISIONS)
        .with_description("Access-policy decisions taken by the proxy.")
        .build()
});

/// Name of the counter of audit lines (exposed to Prometheus as
/// `proxyauth_audit_events_total`). Labels: `event` and `outcome`, both closed
/// lists (RFC 0008); never a user or a path.
pub const AUDIT_EVENTS: &str = "proxyauth.audit.events";

static AUDIT_EVENTS_COUNTER: LazyLock<Counter<u64>> = LazyLock::new(|| {
    opentelemetry::global::meter("proxyauthk8s")
        .u64_counter(AUDIT_EVENTS)
        .with_description("Audit lines written, by event and outcome.")
        .build()
});

/// Count one audit line.
pub fn record_audit_event(event: &'static str, outcome: &'static str) {
    AUDIT_EVENTS_COUNTER.add(
        1,
        &[
            KeyValue::new("event", event),
            KeyValue::new("outcome", outcome),
        ],
    );
}

/// Count one policy decision.
pub fn record_policy_decision(cluster: &str, policy: &'static str, decision: &'static str) {
    POLICY_DECISIONS_COUNTER.add(
        1,
        &[
            KeyValue::new("cluster", cluster.to_string()),
            KeyValue::new("policy", policy),
            KeyValue::new("decision", decision),
        ],
    );
}

/// Bucket boundaries, in seconds, recommended by the HTTP semantic conventions.
const DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// The registry the `/management/metrics` endpoint renders. `None` (or no
/// such request extension at all) disables the endpoint: it then answers `404`.
#[derive(Clone, Default)]
pub struct PrometheusRegistry(pub Option<prometheus::Registry>);

/// Prometheus metrics in the text exposition format 0.0.4.
///
/// Only served when `METRICS_PROMETHEUS_ENABLED` is set; `404` otherwise. The
/// same metrics are exported over OTLP when a collector is configured.
#[utoipa::path(
    get,
    path = "/metrics",
    tag = "health",
    responses(
        (status = 200, description = "Metrics in the Prometheus text exposition format 0.0.4.", content_type = "text/plain", body = String),
        (status = 404, description = "The Prometheus endpoint is disabled (`METRICS_PROMETHEUS_ENABLED` is not set)."),
    )
)]
#[instrument(name = "metrics", level = "debug", skip(req))]
pub async fn prometheus_metrics(req: Parts) -> Response {
    let Some(registry) = req
        .extensions
        .get::<PrometheusRegistry>()
        .and_then(|registry| registry.0.as_ref())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut body = Vec::new();
    if let Err(e) = TextEncoder::new().encode(&registry.gather(), &mut body) {
        warn!(error = %e, "Failed to encode the Prometheus metrics");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

/// Middleware recording [`HTTP_SERVER_REQUEST_DURATION`] (seconds) for every
/// request, with `http.request.method`, `http.route` (the matched pattern, so
/// cardinality stays bounded) and `http.response.status_code`.
#[derive(Clone)]
pub struct HttpMetrics {
    duration: Histogram<f64>,
}

impl HttpMetrics {
    /// Create the instrument on `meter` (e.g. `opentelemetry::global::meter`,
    /// called once the meter provider is installed).
    #[must_use]
    pub fn new(meter: &Meter) -> Self {
        let duration = meter
            .f64_histogram(HTTP_SERVER_REQUEST_DURATION)
            .with_unit("s")
            .with_description("Duration of HTTP server requests.")
            .with_boundaries(DURATION_BUCKETS.to_vec())
            .build();
        Self { duration }
    }
}

/// The route a request matched, carried on its response from inside the
/// router (where [`MatchedPath`] is known) to [`record_duration`] outside it,
/// which also sees the requests no route matched.
#[derive(Clone)]
pub(crate) struct MatchedRoute(pub(crate) String);

/// `route_layer` middleware: copy the matched route onto the response.
pub async fn carry_matched_route(req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|path| actix_style_route(path.as_str()));
    let mut response = next.run(req).await;
    if let Some(route) = route {
        response.extensions_mut().insert(MatchedRoute(route));
    }
    response
}

/// The proxy's catch-all as actix wrote it (`{path:.*}`), so the `http.route`
/// label keeps its value across the move to axum (`{*path}`).
fn actix_style_route(route: &str) -> String {
    route.replace("{*path}", "{path:.*}")
}

/// Middleware recording [`HTTP_SERVER_REQUEST_DURATION`] for every request.
pub async fn record_duration(
    State(metrics): State<HttpMetrics>,
    req: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().as_str().to_owned();
    let response = next.run(req).await;
    let mut attributes = vec![
        KeyValue::new("http.request.method", method),
        KeyValue::new(
            "http.response.status_code",
            i64::from(response.status().as_u16()),
        ),
    ];
    if let Some(MatchedRoute(route)) = response.extensions().get::<MatchedRoute>() {
        attributes.push(KeyValue::new("http.route", route.clone()));
    }
    metrics
        .duration
        .record(start.elapsed().as_secs_f64(), &attributes);
    response
}

#[cfg(test)]
mod tests {
    use super::actix_style_route;

    #[test]
    fn the_catch_all_keeps_its_label() {
        assert_eq!(
            actix_style_route("/clusters/{ns}/{cluster}/{*path}"),
            "/clusters/{ns}/{cluster}/{path:.*}"
        );
        assert_eq!(
            actix_style_route("/management/health"),
            "/management/health"
        );
    }
}
