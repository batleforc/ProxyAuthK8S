//! Tracing, logging, and OpenTelemetry setup for the server binary.
//!
//! Wires `tracing` subscribers to stdout (plain text or JSON lines) and, when
//! configured, an OTLP exporter, optionally exposes the metrics as a Prometheus
//! registry for a pull endpoint, and provides the start/shutdown hooks the
//! server calls at boot.

// https://github.com/open-telemetry/opentelemetry-rust/blob/main/opentelemetry-otlp/examples/basic-otlp/src/main.rs#L33

use std::sync::OnceLock;

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider;
use opentelemetry::{InstrumentationScope, KeyValue, global};
use opentelemetry_otlp::ExporterBuildError;
#[cfg(feature = "metrics")]
use opentelemetry_otlp::MetricExporter;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithTonicConfig;
use opentelemetry_otlp::tonic_types::metadata;
#[cfg(feature = "metrics")]
use opentelemetry_sdk::metrics::SdkMeterProvider;
#[cfg(feature = "metrics")]
use opentelemetry_sdk::metrics::{Instrument, Stream};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing::Subscriber;
use tracing_subscriber::Registry;
use tracing_subscriber::filter::ParseError;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt};

/// Everything that can stop [`start_tracing`] from installing the subscriber.
///
/// A failing OTLP exporter is *not* one of them: tracing then falls back to
/// stdout-only logging and logs a warning instead.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("invalid tracing filter directive: {0}")]
    Directive(#[from] ParseError),
    #[error("failed to install the global tracing subscriber: {0}")]
    Subscriber(#[from] tracing::subscriber::SetGlobalDefaultError),
}
/// Shape of the stdout log lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, one line per event (the historical format).
    #[default]
    Text,
    /// One JSON object per line: `timestamp` (RFC 3339), `level`, `target`,
    /// `threadName`, `fields`, the current `span` and the `spans` list.
    Json,
}

#[derive(Clone)]
pub struct Context {
    pub pod_name: String,
    pub service_name: String,
    /// Format of the stdout logs.
    pub log_format: LogFormat,
    /// Also feed the metrics into a Prometheus registry (returned in
    /// [`TracingOutput::prometheus_registry`]) for a pull endpoint. The OTLP
    /// export keeps working alongside.
    pub prometheus_enabled: bool,
}

pub struct TracingOutput {
    pub tracer_provider: SdkTracerProvider,
    #[cfg(feature = "metrics")]
    pub meter_provider: SdkMeterProvider,
    /// The registry the meter provider exports into, when
    /// [`Context::prometheus_enabled`] is set (and the `metrics` feature is on)
    /// and the Prometheus exporter could be registered. Render it with
    /// `prometheus::TextEncoder`.
    pub prometheus_registry: Option<prometheus::Registry>,
}

pub mod helper;

fn get_resource(ctx: &Context) -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| {
            Resource::builder()
                .with_service_name(ctx.service_name.clone())
                .with_attributes(vec![
                    KeyValue::new("service.pod", ctx.pod_name.clone()),
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
                ])
                .build()
        })
        .clone()
}

fn get_metadata(ctx: &Context) -> metadata::MetadataMap {
    let mut metadata = metadata::MetadataMap::new();
    // service_name / pod_name come from env (POD_NAME / HOSTNAME); a value with
    // characters invalid for a gRPC metadata value must not crash the process at
    // boot — skip the offending key instead.
    if let Ok(value) = ctx.service_name.clone().parse() {
        metadata.insert("service.name", value);
    }
    if let Ok(value) = ctx.pod_name.clone().parse() {
        metadata.insert("service.pod", value);
    }
    metadata
}

/// Build the tracer provider. When the OTLP span exporter cannot be built the
/// provider is still returned (without an exporter, so spans stay local) along
/// with the build error, for the caller to report once logging is up.
fn init_traces(ctx: &Context) -> (SdkTracerProvider, Option<ExporterBuildError>) {
    let builder = SdkTracerProvider::builder()
        .with_resource(get_resource(ctx))
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_max_events_per_span(64)
        .with_max_attributes_per_span(16);
    match SpanExporter::builder()
        .with_tonic()
        .with_metadata(get_metadata(ctx))
        .build()
    {
        Ok(exporter) => (builder.with_batch_exporter(exporter).build(), None),
        Err(e) => (builder.build(), Some(e)),
    }
}

#[cfg(feature = "metrics")]
/// The meter provider and what went wrong while building its exporters.
#[cfg(feature = "metrics")]
struct Metrics {
    provider: SdkMeterProvider,
    prometheus_registry: Option<prometheus::Registry>,
    otlp_error: Option<ExporterBuildError>,
    prometheus_error: Option<String>,
}

#[cfg(feature = "metrics")]
/// Build the meter provider; same fallback contract as [`init_traces`]. When
/// `ctx.prometheus_enabled`, a Prometheus reader is attached too, next to the
/// OTLP periodic exporter.
fn init_metrics(ctx: &Context) -> Metrics {
    let mut builder = SdkMeterProvider::builder()
        .with_resource(get_resource(ctx))
        .with_view(|i: &Instrument| {
            if i.name() == "http.server.duration" {
                // Only fails on an invalid stream name; this one is a valid
                // constant, and dropping the view is harmless anyway.
                Stream::builder()
                    .with_name("http.server.duration")
                    .build()
                    .ok()
            } else {
                None
            }
        });
    let mut prometheus_registry = None;
    let mut prometheus_error = None;
    if ctx.prometheus_enabled {
        let registry = prometheus::Registry::new();
        match opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()
        {
            Ok(reader) => {
                builder = builder.with_reader(reader);
                prometheus_registry = Some(registry);
            }
            Err(e) => prometheus_error = Some(e.to_string()),
        }
    }
    let (provider, otlp_error) = match MetricExporter::builder()
        .with_tonic()
        .with_metadata(get_metadata(ctx))
        .build()
    {
        Ok(exporter) => (builder.with_periodic_exporter(exporter).build(), None),
        Err(e) => (builder.build(), Some(e)),
    };
    Metrics {
        provider,
        prometheus_registry,
        otlp_error,
        prometheus_error,
    }
}

/// `RUST_LOG` when set and valid, else `info`, with the noisy transport
/// crates silenced.
fn base_filter(rust_log: Option<&str>) -> Result<EnvFilter, ParseError> {
    Ok(rust_log
        .and_then(|directives| EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| EnvFilter::new("info"))
        .add_directive("hyper=off".parse()?)
        .add_directive("tonic=off".parse()?)
        .add_directive("h2=off".parse()?)
        .add_directive("reqwest=off".parse()?))
}

/// The filter applied to the whole subscriber (and so to the OTLP export).
fn env_filter(rust_log: Option<&str>) -> Result<EnvFilter, ParseError> {
    base_filter(rust_log)
}

/// The filter applied to the stdout layer: same as [`env_filter`] (so
/// `RUST_LOG` now drives stdout too), with the OpenTelemetry SDK's own logs
/// capped at `info`.
fn fmt_filter(rust_log: Option<&str>) -> Result<EnvFilter, ParseError> {
    Ok(base_filter(rust_log)?.add_directive("opentelemetry=info".parse()?))
}

/// The stdout layer, in the requested `format`, writing to `writer`.
fn fmt_layer<S, W>(format: LogFormat, writer: W) -> Box<dyn Layer<S> + Send + Sync>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_target(true)
        .with_writer(writer);
    match format {
        LogFormat::Text => layer.boxed(),
        LogFormat::Json => layer
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .boxed(),
    }
}

/// Install the global `tracing` subscriber (stdout + OpenTelemetry) and the
/// OpenTelemetry tracer/meter providers.
///
/// If an OTLP exporter cannot be built (e.g. a malformed
/// `OTEL_EXPORTER_OTLP_ENDPOINT`), the matching provider is installed without
/// an exporter and a warning is logged: the process keeps running with
/// stdout-only logs instead of crashing at boot.
///
/// # Errors
///
/// Returns [`TraceError`] when a filter directive is invalid or a global
/// subscriber is already installed.
pub fn start_tracing(ctx: &Context) -> Result<TracingOutput, TraceError> {
    global::set_text_map_propagator(TraceContextPropagator::new());

    let mut exporter_errors: Vec<(&str, ExporterBuildError)> = Vec::new();

    // Initialize tracer
    let (tracer_provider, span_err) = init_traces(ctx);
    exporter_errors.extend(span_err.map(|e| ("span", e)));
    global::set_tracer_provider(tracer_provider.clone());

    // Initialize meter
    #[cfg(feature = "metrics")]
    let metrics = init_metrics(ctx);
    #[cfg(feature = "metrics")]
    exporter_errors.extend(metrics.otlp_error.map(|e| ("metric", e)));
    #[cfg(feature = "metrics")]
    let meter_provider = metrics.provider;
    #[cfg(feature = "metrics")]
    global::set_meter_provider(meter_provider.clone());
    #[cfg(feature = "metrics")]
    let (prometheus_registry, prometheus_error) =
        (metrics.prometheus_registry, metrics.prometheus_error);
    #[cfg(not(feature = "metrics"))]
    let (prometheus_registry, prometheus_error): (
        Option<prometheus::Registry>,
        Option<String>,
    ) = (None, None);

    // Set instrumentation scope
    let common_scope_attributes = vec![KeyValue::new("service.framework", "rust")];
    let scope = InstrumentationScope::builder("basic")
        .with_version("1.0")
        .with_attributes(common_scope_attributes)
        .build();
    global::tracer_with_scope(scope.clone());
    #[cfg(feature = "metrics")]
    global::meter_with_scope(scope);

    // Setup subscriber
    #[cfg(feature = "otel")]
    let tracer = tracer_provider.tracer(ctx.service_name.clone());

    let rust_log = std::env::var("RUST_LOG").ok();
    let env_filter = env_filter(rust_log.as_deref())?;
    let filter_fmt = fmt_filter(rust_log.as_deref())?;

    let fmt_layer = fmt_layer(ctx.log_format, std::io::stdout).with_filter(filter_fmt);

    #[cfg(feature = "otel")]
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    #[cfg(feature = "otel")]
    let subscriber = Registry::default()
        .with(env_filter)
        .with(telemetry)
        .with(fmt_layer);

    #[cfg(not(feature = "otel"))]
    let subscriber = Registry::default().with(env_filter).with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)?;

    for (kind, e) in exporter_errors {
        tracing::warn!(
            error = %e,
            "Failed to build the OTLP {kind} exporter, falling back to stdout-only logging"
        );
    }
    if let Some(e) = prometheus_error {
        tracing::warn!(
            error = %e,
            "Failed to register the Prometheus metrics exporter, the metrics endpoint stays disabled"
        );
    }
    if ctx.prometheus_enabled && cfg!(not(feature = "metrics")) {
        tracing::warn!(
            "Prometheus metrics requested but the `metrics` feature is off, the metrics endpoint stays disabled"
        );
    }

    Ok(TracingOutput {
        tracer_provider,
        #[cfg(feature = "metrics")]
        meter_provider,
        prometheus_registry,
    })
}

pub fn shutdown_tracing(tracing_output: TracingOutput) -> Result<(), String> {
    let mut shutdown_errors = Vec::new();
    if let Err(e) = tracing_output.tracer_provider.shutdown() {
        shutdown_errors.push(format!("tracer provider: {e}"));
    }

    #[cfg(feature = "metrics")]
    if let Err(e) = tracing_output.meter_provider.shutdown() {
        shutdown_errors.push(format!("meter provider: {e}"));
    }

    // Return an error if any shutdown failed
    if !shutdown_errors.is_empty() {
        return Err(format!(
            "Failed to shutdown providers:{}",
            shutdown_errors.join("\n")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::{Layer, Registry, fmt::MakeWriter, layer::SubscriberExt};

    use super::{LogFormat, TraceError, env_filter, fmt_filter, fmt_layer};

    #[test]
    fn the_static_filter_directives_parse() {
        assert!(env_filter(None).is_ok());
        assert!(fmt_filter(None).is_ok());
    }

    #[test]
    fn stdout_defaults_to_info_without_rust_log() {
        let filter = fmt_filter(None).unwrap();
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn stdout_follows_rust_log_when_set() {
        let filter = fmt_filter(Some("debug")).unwrap();
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::DEBUG));
        let filter = fmt_filter(Some("warn")).unwrap();
        // `opentelemetry=info` still lifts the hint to `info`.
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
        let rendered = fmt_filter(Some("trace")).unwrap().to_string();
        for noisy in ["hyper=off", "tonic=off", "h2=off", "reqwest=off"] {
            assert!(rendered.contains(noisy), "{rendered}");
        }
    }

    #[test]
    fn an_invalid_rust_log_falls_back_to_info() {
        let filter = fmt_filter(Some("my_crate=loud")).unwrap();
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
        let filter = env_filter(Some("my_crate=loud")).unwrap();
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn the_default_log_format_is_text() {
        assert_eq!(LogFormat::default(), LogFormat::Text);
    }

    /// Captures everything the fmt layer writes.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture(format: LogFormat) -> String {
        let buffer = Buffer::default();
        let subscriber = Registry::default()
            .with(fmt_layer(format, buffer.clone()).with_filter(fmt_filter(None).unwrap()));
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("request", request_id = "abc");
            let _guard = span.enter();
            tracing::info!(cluster = "prod", "proxied");
            tracing::debug!("filtered out at the default level");
        });
        String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn json_format_emits_one_object_per_line() {
        let output = capture(LogFormat::Json);
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 1, "{output}");
        let event: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event["level"], "INFO");
        assert_eq!(event["target"], module_path!());
        assert_eq!(event["fields"]["message"], "proxied");
        assert_eq!(event["fields"]["cluster"], "prod");
        assert_eq!(event["span"]["name"], "request");
        assert_eq!(event["span"]["request_id"], "abc");
        assert_eq!(event["spans"][0]["name"], "request");
        // RFC 3339 UTC, e.g. `2026-10-05T12:34:56.789012Z`.
        let timestamp = event["timestamp"].as_str().unwrap();
        assert_eq!(timestamp.as_bytes()[10], b'T', "{timestamp}");
        assert!(timestamp.ends_with('Z'), "{timestamp}");
    }

    #[test]
    fn text_format_is_not_json() {
        let output = capture(LogFormat::Text);
        assert_eq!(output.lines().count(), 1, "{output}");
        assert!(serde_json::from_str::<serde_json::Value>(output.trim()).is_err());
        assert!(output.contains("proxied"), "{output}");
        assert!(output.contains("\"prod\""), "{output}");
    }

    #[test]
    fn a_directive_parse_error_converts_into_trace_error() {
        let err: TraceError = "target=not_a_level"
            .parse::<tracing_subscriber::filter::Directive>()
            .unwrap_err()
            .into();
        assert!(matches!(err, TraceError::Directive(_)));
        assert!(
            err.to_string()
                .starts_with("invalid tracing filter directive")
        );
    }
}
