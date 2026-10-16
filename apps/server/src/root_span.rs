//! Root span for every HTTP request, linked to the caller's trace.
//!
//! `tracing-actix-web` only propagates the incoming trace context through its
//! `opentelemetry_0_x` features, which lag behind the `opentelemetry` releases
//! this workspace tracks. Mixing versions compiles but silently drops the
//! link: its `set_parent` targets a `tracing-opentelemetry` layer that is not
//! the one registered. The extraction is done here instead, against the same
//! `opentelemetry` / `tracing-opentelemetry` versions as `trace::start_tracing`.

use actix_web::{
    Error,
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    http::header::HeaderMap,
};
use opentelemetry::{propagation::Extractor, trace::TraceContextExt as _};
use tracing::Span;
use tracing_actix_web::{DefaultRootSpanBuilder, RootSpanBuilder, root_span};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

/// [`RootSpanBuilder`] that parents the request span on the `traceparent`
/// sent by the caller (through the global propagator) and records `trace_id`.
pub struct OtelRootSpanBuilder;

impl RootSpanBuilder for OtelRootSpanBuilder {
    fn on_request_start(request: &ServiceRequest) -> Span {
        let span = root_span!(request);
        let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(request.headers()))
        });
        // Only fails once the span has been entered, which cannot have
        // happened yet: the span was just created.
        let _ = span.set_parent(parent);
        // The caller's trace id when one was propagated, the new one otherwise.
        let trace_id = span.context().span().span_context().trace_id();
        span.record("trace_id", tracing::field::display(trace_id));
        span
    }

    fn on_request_end<B: MessageBody>(span: Span, outcome: &Result<ServiceResponse<B>, Error>) {
        DefaultRootSpanBuilder::on_request_end(span, outcome);
    }
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(actix_web::http::header::HeaderName::as_str)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{App, HttpResponse, test, web};
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
    use tracing_actix_web::TracingLogger;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::*;

    async fn current_trace_id() -> HttpResponse {
        let trace_id = Span::current().context().span().span_context().trace_id();
        HttpResponse::Ok().body(trace_id.to_string())
    }

    async fn trace_id_for(traceparent: Option<&str>) -> String {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        let provider = SdkTracerProvider::builder().build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let _guard = tracing::subscriber::set_default(subscriber);

        let app = test::init_service(
            App::new()
                .wrap(TracingLogger::<OtelRootSpanBuilder>::new())
                .route("/", web::get().to(current_trace_id)),
        )
        .await;
        let mut request = test::TestRequest::get().uri("/");
        if let Some(traceparent) = traceparent {
            request = request.insert_header(("traceparent", traceparent));
        }
        let body = test::call_and_read_body(&app, request.to_request()).await;
        String::from_utf8(body.to_vec()).expect("utf-8 trace id")
    }

    #[actix_web::test]
    async fn the_request_span_joins_the_callers_trace() {
        let trace_id = trace_id_for(Some(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ))
        .await;
        assert_eq!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[actix_web::test]
    async fn without_traceparent_a_new_trace_is_started() {
        let trace_id = trace_id_for(None).await;
        assert_ne!(trace_id, opentelemetry::TraceId::INVALID.to_string());
    }
}
