//! Root span for every HTTP request, linked to the caller's trace, and the
//! `x-request-id` of its response.
//!
//! The span keeps the fields `tracing-actix-web` gave it before the move to
//! axum (`http.method`, `http.route`, `request_id`, `trace_id`…), so the logs
//! and the audit lines nested under it keep their shape. The incoming trace
//! context is extracted against the same `opentelemetry` /
//! `tracing-opentelemetry` versions as `trace::start_tracing`.

use axum::{
    extract::{ConnectInfo, Request},
    http::{HeaderMap, HeaderName, HeaderValue, Version, header},
    middleware::Next,
    response::Response,
};
use opentelemetry::propagation::Extractor;
use tracing::{Instrument as _, field::Empty};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::{
    cluster::redirect::{audit, forwarded::throttle_client_ip},
    helper::forwarded_for,
    metrics::MatchedRoute,
};

/// Middleware: run the request inside its root span, then record the route
/// and the status on it, and name the request in `x-request-id`.
pub async fn root_span(req: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4();
    let span = request_span(&req, &request_id);
    // Read by every audit line of the request (RFC 0008): the proxy's own
    // request id, never a client's `x-request-id`, and the client address as
    // bans see it.
    let peer = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let meta = audit::RequestMeta {
        request_id: request_id.to_string(),
        client_ip: throttle_client_ip(forwarded_for(req.headers()), peer),
    };
    let mut response = audit::scope(meta, next.run(req).instrument(span.clone())).await;

    if let Some(MatchedRoute(route)) = response.extensions().get::<MatchedRoute>() {
        span.record("http.route", route.as_str());
        span.record("otel.name", format!("HTTP {route}"));
    }
    let status = response.status();
    span.record("http.status_code", status.as_u16());
    span.record(
        "otel.status_code",
        if status.is_server_error() {
            "ERROR"
        } else {
            "OK"
        },
    );
    if let Ok(value) = HeaderValue::from_str(&request_id.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-request-id"), value);
    }
    response
}

fn request_span(req: &Request, request_id: &uuid::Uuid) -> tracing::Span {
    let headers = req.headers();
    let header = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|ip| ip.trim().to_string())
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<std::net::SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip().to_string())
        })
        .unwrap_or_default();
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().scheme_str().map(str::to_string))
        .unwrap_or_else(|| "http".to_string());
    let host = req
        .uri()
        .authority()
        .map(ToString::to_string)
        .unwrap_or_else(|| header(header::HOST));
    let flavor = match req.version() {
        Version::HTTP_09 => "0.9",
        Version::HTTP_10 => "1.0",
        Version::HTTP_2 => "2.0",
        Version::HTTP_3 => "3.0",
        _ => "1.1",
    };
    let target = req
        .uri()
        .path_and_query()
        .map_or_else(|| req.uri().path().to_string(), ToString::to_string);
    let span = tracing::info_span!(
        "HTTP request",
        http.method = %req.method(),
        http.route = Empty,
        http.flavor = flavor,
        http.scheme = %scheme,
        http.host = %host,
        http.client_ip = %client_ip,
        http.user_agent = %header(header::USER_AGENT),
        http.target = %target,
        http.status_code = Empty,
        otel.name = %format!("HTTP {}", req.method()),
        otel.kind = "server",
        otel.status_code = Empty,
        request_id = %request_id,
        trace_id = Empty,
        exception.message = Empty,
        exception.details = Empty,
    );
    let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(headers))
    });
    // Only fails once the span has been entered, which cannot have
    // happened yet: the span was just created.
    let _ = span.set_parent(parent);
    // The caller's trace id when one was propagated, the new one otherwise;
    // none at all without an OpenTelemetry layer.
    if let Some(trace_id) = trace::helper::trace_id_of(&span) {
        span.record("trace_id", tracing::field::display(trace_id));
    }
    span
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request, middleware, routing::get};
    use http_body_util::BodyExt as _;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
    use tower::ServiceExt as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::root_span;

    async fn current_trace_id() -> String {
        trace::helper::get_trace_id()
            .unwrap_or(opentelemetry::TraceId::INVALID)
            .to_string()
    }

    async fn trace_id_for(traceparent: Option<&str>) -> String {
        // The propagator is process-global and never reset: install it once,
        // the same one `trace::start_tracing` installs in production.
        static PROPAGATOR: std::sync::Once = std::sync::Once::new();
        PROPAGATOR.call_once(|| {
            opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        });
        let provider = SdkTracerProvider::builder().build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let _guard = tracing::subscriber::set_default(subscriber);

        let app = Router::new()
            .route("/", get(current_trace_id))
            .layer(middleware::from_fn(root_span));
        let mut request = Request::get("/");
        if let Some(traceparent) = traceparent {
            request = request.header("traceparent", traceparent);
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers()["x-request-id"].len(), 36);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(body.to_vec()).expect("utf-8 trace id")
    }

    /// T8 (RFC 0008): the audit line carries the proxy's request id, the one
    /// the response names, never the id the client sent.
    #[tokio::test]
    async fn an_audit_line_carries_the_proxys_request_id() {
        use crate::cluster::redirect::audit::{AuditEvent, Event, Outcome, capture};

        capture::start();
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    AuditEvent::new(Event::Request, Outcome::Allowed)
                        .cluster("root-span", "t8")
                        .emit();
                }),
            )
            .layer(middleware::from_fn(root_span));
        let response = app
            .oneshot(
                Request::get("/")
                    .header("x-request-id", "forged")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let answered = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_string();
        let line = capture::lines()
            .into_iter()
            .find(|line| line["ns"] == "root-span" && line["cluster"] == "t8")
            .expect("the audit line");
        assert_eq!(line["request_id"], answered.as_str());
        assert_ne!(line["request_id"], "forged");
    }

    #[tokio::test]
    async fn the_request_span_joins_the_callers_trace() {
        let trace_id = trace_id_for(Some(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ))
        .await;
        assert_eq!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[tokio::test]
    async fn without_traceparent_a_new_trace_is_started() {
        let trace_id = trace_id_for(None).await;
        assert_ne!(trace_id, opentelemetry::TraceId::INVALID.to_string());
    }
}
