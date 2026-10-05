//! Logs (JSON lines on stdout, level from `RUST_LOG`) and OpenTelemetry
//! metrics (OTLP gRPC). Metric names are documented in the ADR
//! (`docs/adr/0001-mqtt-ingest-services.md`).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, MeterProvider as _};
use opentelemetry_otlp::{MetricExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use tracing_subscriber::EnvFilter;

use crate::config::TelemetryConfig;
use crate::topic::Kind;

pub fn init_logging() {
    tracing_subscriber::fmt()
        .json()
        .with_current_span(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
}

/// Flushes metrics on drop.
pub struct Telemetry {
    provider: Option<SdkMeterProvider>,
    pub metrics: Metrics,
}

impl Telemetry {
    pub fn init(service: &'static str, instance: &str, config: &TelemetryConfig) -> Result<Self> {
        let Some(endpoint) = &config.otlp_endpoint else {
            tracing::warn!("telemetry.otlpEndpoint is not set; metrics are off");
            return Ok(Self { provider: None, metrics: Metrics::disabled() });
        };
        let exporter = MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(Duration::from_secs(5))
            .build()
            .context("creating the OTLP metric exporter")?;
        let reader = PeriodicReader::builder(exporter)
            .with_interval(Duration::from_secs(config.export_interval_secs.max(1)))
            .build();
        let resource = Resource::builder()
            .with_service_name(service)
            .with_attribute(KeyValue::new("service.instance.id", instance.to_owned()))
            .build();
        let provider = SdkMeterProvider::builder().with_reader(reader).with_resource(resource).build();
        let metrics = Metrics::new(&provider.meter("asimov-ingest"));
        Ok(Self { provider: Some(provider), metrics })
    }
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take()
            && let Err(error) = provider.shutdown()
        {
            tracing::warn!(error = %error, "flushing metrics failed");
        }
    }
}

/// Seconds, from 1 ms to 2 min.
const SECONDS_BUCKETS: [f64; 14] = [0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 10.0, 30.0, 120.0];
const BATCH_BUCKETS: [f64; 10] = [1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 5000.0];

/// All instruments, cheap to clone. `disabled()` records nothing (tests,
/// or no OTLP endpoint).
#[derive(Clone)]
pub struct Metrics(Arc<Instruments>);

struct Instruments {
    received: Counter<u64>,
    rejected: Counter<u64>,
    mqtt_connected: Gauge<u64>,
    sink_connected: Gauge<u64>,
    reconnects: Counter<u64>,
    sink_writes: Counter<u64>,
    stale_skipped: Counter<u64>,
    write_duration: Histogram<f64>,
    batch_size: Histogram<u64>,
    buffer_depth: Gauge<u64>,
    buffer_dropped: Counter<u64>,
    latency: Histogram<f64>,
    task_restarts: Counter<u64>,
}

fn labels(site: &str, kind: Kind) -> [KeyValue; 2] {
    [KeyValue::new("site", site.to_owned()), KeyValue::new("kind", kind.as_str())]
}

impl Metrics {
    pub fn disabled() -> Self {
        Self::new(&opentelemetry::metrics::noop::NoopMeterProvider::new().meter("disabled"))
    }

    pub fn new(meter: &Meter) -> Self {
        Self(Arc::new(Instruments {
            received: meter.u64_counter("ingest_mqtt_messages_received").with_description("MQTT messages received").build(),
            rejected: meter
                .u64_counter("ingest_mqtt_messages_rejected")
                .with_description("MQTT messages not stored, by reason")
                .build(),
            mqtt_connected: meter.u64_gauge("ingest_mqtt_connected").with_description("1 while connected to the broker").build(),
            sink_connected: meter.u64_gauge("ingest_sink_connected").with_description("1 while connected to the sink").build(),
            reconnects: meter.u64_counter("ingest_reconnects").with_description("Reconnects, by component").build(),
            sink_writes: meter.u64_counter("ingest_sink_writes").with_description("Rows or calls written, by result").build(),
            stale_skipped: meter
                .u64_counter("ingest_sink_stale_skipped")
                .with_description("Messages dropped because a newer one for the same AGV was already buffered")
                .build(),
            write_duration: meter
                .f64_histogram("ingest_sink_write_duration")
                .with_unit("s")
                .with_boundaries(SECONDS_BUCKETS.to_vec())
                .build(),
            batch_size: meter.u64_histogram("ingest_sink_batch_size").with_boundaries(BATCH_BUCKETS.to_vec()).build(),
            buffer_depth: meter.u64_gauge("ingest_buffer_depth").with_description("Messages waiting for the sink").build(),
            buffer_dropped: meter
                .u64_counter("ingest_buffer_dropped")
                .with_description("Messages dropped because the buffer was full")
                .build(),
            latency: meter
                .f64_histogram("ingest_end_to_end_latency")
                .with_description("Time from the VDA 5050 header timestamp to the sink write")
                .with_unit("s")
                .with_boundaries(SECONDS_BUCKETS.to_vec())
                .build(),
            task_restarts: meter.u64_counter("ingest_task_restarts").with_description("Supervised task restarts").build(),
        }))
    }

    pub fn received(&self, site: &str, kind: Kind) {
        self.0.received.add(1, &labels(site, kind));
    }

    /// `site` and `kind` are unknown when the topic itself is rejected.
    pub fn rejected(&self, site: Option<&str>, kind: Option<Kind>, reason: &'static str) {
        self.0.rejected.add(
            1,
            &[
                KeyValue::new("site", site.unwrap_or("unknown").to_owned()),
                KeyValue::new("kind", kind.map_or("unknown", Kind::as_str)),
                KeyValue::new("reason", reason),
            ],
        );
    }

    pub fn mqtt_connected(&self, connected: bool) {
        self.0.mqtt_connected.record(connected.into(), &[]);
    }

    pub fn sink_connected(&self, connected: bool) {
        self.0.sink_connected.record(connected.into(), &[]);
    }

    pub fn reconnected(&self, component: &'static str) {
        self.0.reconnects.add(1, &[KeyValue::new("component", component)]);
    }

    pub fn sink_written(&self, site: &str, kind: Kind, result: &'static str, count: u64) {
        let [site, kind] = labels(site, kind);
        self.0.sink_writes.add(count, &[site, kind, KeyValue::new("result", result)]);
    }

    pub fn stale_skipped(&self, site: &str, kind: Kind) {
        self.0.stale_skipped.add(1, &labels(site, kind));
    }

    pub fn write_duration(&self, kind: Kind, duration: Duration) {
        self.0.write_duration.record(duration.as_secs_f64(), &[KeyValue::new("kind", kind.as_str())]);
    }

    pub fn batch_size(&self, kind: Kind, size: u64) {
        self.0.batch_size.record(size, &[KeyValue::new("kind", kind.as_str())]);
    }

    pub fn buffer_depth(&self, depth: u64) {
        self.0.buffer_depth.record(depth, &[]);
    }

    pub fn buffer_dropped(&self, site: &str, kind: Kind) {
        self.0.buffer_dropped.add(1, &labels(site, kind));
    }

    /// `sent_at` in microseconds since the Unix epoch.
    pub fn latency(&self, site: &str, kind: Kind, sent_at: i64) {
        let now = chrono::Utc::now().timestamp_micros();
        // Clock skew between AGV and service can make this negative; clamp.
        let seconds = (now - sent_at).max(0) as f64 / 1e6;
        self.0.latency.record(seconds, &labels(site, kind));
    }

    pub fn task_restarted(&self, task: &'static str) {
        self.0.task_restarts.add(1, &[KeyValue::new("task", task)]);
    }
}

/// Container hostname, used in the MQTT client id and as `service.instance.id`.
pub fn instance_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().into_owned())
}
