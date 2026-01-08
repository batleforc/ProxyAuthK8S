// https://github.com/open-telemetry/opentelemetry-rust/blob/main/opentelemetry-otlp/examples/basic-otlp/src/main.rs#L33

use std::sync::OnceLock;

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use opentelemetry_otlp::tonic_types::metadata;
#[cfg(feature = "metrics")]
use opentelemetry_otlp::MetricExporter;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithTonicConfig;
#[cfg(feature = "metrics")]
use opentelemetry_sdk::metrics::SdkMeterProvider;
#[cfg(feature = "metrics")]
use opentelemetry_sdk::metrics::{Instrument, Stream};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler};
use opentelemetry_sdk::{trace::SdkTracerProvider, Resource};
use tracing_subscriber::Registry;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};
#[derive(Clone)]
pub struct Context {
    pub pod_name: String,
    pub service_name: String,
}

pub struct TracingOutput {
    pub tracer_provider: SdkTracerProvider,
    #[cfg(feature = "metrics")]
    pub meter_provider: SdkMeterProvider,
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
    metadata.insert("service.name", ctx.service_name.clone().parse().unwrap());
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
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_max_events_per_span(64)
        .with_max_attributes_per_span(16)
        .with_batch_exporter(exporter)
        .build()
}

#[cfg(feature = "metrics")]
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

pub fn start_tracing(ctx: &Context) -> TracingOutput {
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Initialize tracer
    let tracer_provider = init_traces(&ctx.clone());
    global::set_tracer_provider(tracer_provider.clone());

    // Initialize meter
    #[cfg(feature = "metrics")]
    let meter_provider = init_metrics(&ctx.clone());
    #[cfg(feature = "metrics")]
    global::set_meter_provider(meter_provider.clone());

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

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("tonic=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap());

    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=info".parse().unwrap());

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_filter(filter_fmt);

    #[cfg(feature = "otel")]
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    #[cfg(feature = "otel")]
    let subscriber = Registry::default()
        .with(env_filter)
        .with(telemetry)
        .with(fmt_layer);

    #[cfg(not(feature = "otel"))]
    let subscriber = Registry::default().with(env_filter).with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to install `tracing` subscriber.");

    TracingOutput {
        tracer_provider,
        #[cfg(feature = "metrics")]
        meter_provider,
    }
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
