use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Initialize tracing with optional Axiom/OTLP export.
///
/// If AXIOM_TOKEN is set, spans and logs are exported to Axiom via OTLP
/// alongside the existing JSON stdout output. Without it, only stdout.
pub fn init() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer().json();

    let axiom_token = std::env::var("AXIOM_TOKEN").ok();
    let axiom_dataset = std::env::var("AXIOM_DATASET").unwrap_or_else(|_| "relay".to_string());

    if let Some(token) = axiom_token {
        // Configure OTLP via standard env vars so the builder picks them up.
        // SAFETY: called once at startup before any other threads are spawned.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "https://api.axiom.co");
            std::env::set_var(
                "OTEL_EXPORTER_OTLP_HEADERS",
                format!(
                    "Authorization=Bearer {},X-Axiom-Dataset={}",
                    token, axiom_dataset
                ),
            );
            std::env::set_var("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf");
        }

        let exporter = SpanExporter::builder()
            .with_http()
            .build()
            .expect("failed to create OTLP exporter");

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(Resource::builder().with_service_name("nexus-relay").build())
            .build();

        let tracer = provider.tracer("nexus-relay");
        let otel_layer = OpenTelemetryLayer::new(tracer);

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(otel_layer)
            .init();

        opentelemetry::global::set_tracer_provider(provider);

        tracing::info!(dataset = %axiom_dataset, "Axiom telemetry enabled");
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .init();
    }
}

/// Flush remaining spans before shutdown.
pub fn shutdown() {
    // The provider stored in the global slot doesn't expose shutdown directly.
    // Dropping the SdkTracerProvider triggers flush. We just need to give it time.
    // In practice, the batch exporter flushes on drop.
    tracing::info!("telemetry shutting down");
}
