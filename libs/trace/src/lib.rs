// https://github.com/open-telemetry/opentelemetry-rust/blob/main/opentelemetry-otlp/examples/basic-otlp/src/main.rs#L33

use std::sync::OnceLock;

use opentelemetry::{global, InstrumentationScope, KeyValue};
#[cfg(feature = "log")]
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::tonic_types::metadata;
#[cfg(feature = "log")]
use opentelemetry_otlp::LogExporter;
use opentelemetry_otlp::WithTonicConfig;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::metrics::{Instrument, Stream};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider, metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource,
};
#[cfg(feature = "log")]
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};
#[derive(Clone)]
pub struct Context {
    pub pod_name: String,
}

pub mod helper;

fn get_resource(ctx: &Context) -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| {
            Resource::builder()
                .with_service_name("proxyauthk8s")
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
    metadata.insert("service.name", "proxyauthk8s".parse().unwrap());
    metadata.insert("service.pod", ctx.pod_name.clone().parse().unwrap());
    metadata
}

fn init_traces(ctx: &Context) -> SdkTracerProvider {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_metadata(get_metadata(ctx))
        .build()
        .expect("Failed to create span exporter");
    SdkTracerProvider::builder()
        .with_resource(get_resource(ctx))
        .with_batch_exporter(exporter)
        .build()
}

fn init_metrics(ctx: &Context) -> SdkMeterProvider {
    let exporter = MetricExporter::builder()
        .with_tonic()
        .with_metadata(get_metadata(ctx))
        .build()
        .expect("Failed to create metric exporter");

    SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(get_resource(ctx))
        .with_view(|i: &Instrument| {
            if i.name() == "http.server.duration" {
                Some(
                    Stream::builder()
                        .with_name("http.server.duration")
                        .build()
                        .unwrap(),
                )
            } else {
                None
            }
        })
        .build()
}

#[cfg(feature = "log")]
fn init_logs(ctx: &Context) -> SdkLoggerProvider {
    let exporter = LogExporter::builder()
        .with_tonic()
        .with_metadata(get_metadata(ctx))
        .build()
        .expect("Failed to create log exporter");

    SdkLoggerProvider::builder()
        .with_resource(get_resource(ctx))
        .with_batch_exporter(exporter)
        .build()
}

pub fn start_tracing(
    ctx: &Context,
) -> (
    Option<SdkLoggerProvider>,
    SdkTracerProvider,
    SdkMeterProvider,
) {
    #[cfg(feature = "log")]
    let logger_provider = {
        let logger_provider = init_logs(&ctx.clone());
        let otel_layer = OpenTelemetryTracingBridge::new(&logger_provider);

        let filter_otel = EnvFilter::new("info")
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("tonic=off".parse().unwrap())
            .add_directive("h2=off".parse().unwrap())
            .add_directive("reqwest=off".parse().unwrap());
        let otel_layer = otel_layer.with_filter(filter_otel);

        let filter_fmt =
            EnvFilter::new("info").add_directive("opentelemetry=info".parse().unwrap());
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_thread_names(true)
            .with_filter(filter_fmt);

        tracing_subscriber::registry()
            .with(otel_layer)
            .with(fmt_layer)
            .init();
        logger_provider
    };

    let tracer_provider = init_traces(&ctx.clone());
    global::set_tracer_provider(tracer_provider.clone());

    let meter_provider = init_metrics(&ctx.clone());
    global::set_meter_provider(meter_provider.clone());

    let common_scope_attributes = vec![KeyValue::new("service.framework", "rust")];
    let scope = InstrumentationScope::builder("basic")
        .with_version("1.0")
        .with_attributes(common_scope_attributes)
        .build();

    global::tracer_with_scope(scope.clone());
    global::meter_with_scope(scope);

    #[cfg(feature = "log")]
    return (Some(logger_provider), tracer_provider, meter_provider);
    #[cfg(not(feature = "log"))]
    (None, tracer_provider, meter_provider)
}

pub fn shutdown_tracing(
    logger_provider: Option<SdkLoggerProvider>,
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
) -> Result<(), String> {
    let mut shutdown_errors = Vec::new();
    if let Err(e) = tracer_provider.shutdown() {
        shutdown_errors.push(format!("tracer provider: {e}"));
    }

    if let Err(e) = meter_provider.shutdown() {
        shutdown_errors.push(format!("meter provider: {e}"));
    }
    if let Some(logger_provider) = logger_provider {
        if let Err(e) = logger_provider.shutdown() {
            shutdown_errors.push(format!("logger provider: {e}"));
        }
    }

    // Return an error if any shutdown failed
    if !shutdown_errors.is_empty() {
        return Err(format!(
            "Failed to shutdown providers:{}",
            shutdown_errors.join("\n")
        )
        .into());
    }
    Ok(())
}
