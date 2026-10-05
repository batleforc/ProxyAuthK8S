//! Prometheus pull endpoint (`GET /management/metrics`) and the HTTP server
//! request-duration instrument it (and the OTLP export) reports.
//!
//! The metrics are recorded through OpenTelemetry; when
//! `METRICS_PROMETHEUS_ENABLED` is set, the `trace` crate also attaches a
//! Prometheus reader to the meter provider and hands its registry to the app
//! as [`PrometheusRegistry`] application data.

use std::{
    future::{Ready, ready},
    rc::Rc,
    time::Instant,
};

use actix_web::{
    Error, HttpResponse, Responder,
    body::MessageBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    get,
    http::header,
    web,
};
use futures_util::future::LocalBoxFuture;
use opentelemetry::{
    KeyValue,
    metrics::{Histogram, Meter},
};
use prometheus::{Encoder, TextEncoder};
use tracing::{instrument, warn};

/// `Content-Type` of the Prometheus text exposition format 0.0.4.
pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Name of the HTTP server request-duration histogram (OpenTelemetry semantic
/// conventions; exposed to Prometheus as `http_server_request_duration_seconds`).
pub const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";

/// Bucket boundaries, in seconds, recommended by the HTTP semantic conventions.
const DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// The registry the `/management/metrics` endpoint renders. `None` (or no
/// such app data at all) disables the endpoint: it then answers `404`.
#[derive(Clone, Default)]
pub struct PrometheusRegistry(pub Option<prometheus::Registry>);

/// Prometheus metrics in the text exposition format 0.0.4.
///
/// Only served when `METRICS_PROMETHEUS_ENABLED` is set; `404` otherwise. The
/// same metrics are exported over OTLP when a collector is configured.
#[utoipa::path(
    tag = "health",
    responses(
        (status = 200, description = "Metrics in the Prometheus text exposition format 0.0.4.", content_type = "text/plain", body = String),
        (status = 404, description = "The Prometheus endpoint is disabled (`METRICS_PROMETHEUS_ENABLED` is not set)."),
    )
)]
#[get("/metrics")]
#[instrument(name = "metrics", level = "debug", skip(registry))]
pub async fn prometheus_metrics(registry: Option<web::Data<PrometheusRegistry>>) -> impl Responder {
    let Some(registry) = registry.as_ref().and_then(|data| data.0.as_ref()) else {
        return HttpResponse::NotFound().finish();
    };
    let mut body = Vec::new();
    if let Err(e) = TextEncoder::new().encode(&registry.gather(), &mut body) {
        warn!(error = %e, "Failed to encode the Prometheus metrics");
        return HttpResponse::InternalServerError().finish();
    }
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE))
        .body(body)
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

impl<S, B> Transform<S, ServiceRequest> for HttpMetrics
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = HttpMetricsMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(HttpMetricsMiddleware {
            service: Rc::new(service),
            duration: self.duration.clone(),
        }))
    }
}

/// The service produced by [`HttpMetrics`].
pub struct HttpMetricsMiddleware<S> {
    service: Rc<S>,
    duration: Histogram<f64>,
}

impl<S, B> Service<ServiceRequest> for HttpMetricsMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    actix_web::dev::forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let start = Instant::now();
        let method = req.method().as_str().to_owned();
        let service = Rc::clone(&self.service);
        let duration = self.duration.clone();
        Box::pin(async move {
            let result = service.call(req).await;
            let (route, status) = match &result {
                Ok(res) => (
                    res.request().match_pattern(),
                    res.response().status().as_u16(),
                ),
                Err(e) => (None, e.as_response_error().status_code().as_u16()),
            };
            let mut attributes = vec![
                KeyValue::new("http.request.method", method),
                KeyValue::new("http.response.status_code", i64::from(status)),
            ];
            if let Some(route) = route {
                attributes.push(KeyValue::new("http.route", route));
            }
            duration.record(start.elapsed().as_secs_f64(), &attributes);
            result
        })
    }
}
