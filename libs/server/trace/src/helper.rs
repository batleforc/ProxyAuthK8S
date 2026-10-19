use opentelemetry::TraceId;

/// Trace id of the current span, `None` when no OpenTelemetry layer gave it one.
#[must_use]
pub fn get_trace_id() -> Option<TraceId> {
    trace_id_of(&tracing::Span::current())
}

/// Trace id of `span`, `None` when no OpenTelemetry layer gave it one.
#[must_use]
pub fn trace_id_of(span: &tracing::Span) -> Option<TraceId> {
    use opentelemetry::trace::TraceContextExt as _;
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    let trace_id = span.context().span().span_context().trace_id();
    (trace_id != TraceId::INVALID).then_some(trace_id)
}
