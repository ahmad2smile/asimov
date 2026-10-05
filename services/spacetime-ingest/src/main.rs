//! MQTT -> SpacetimeDB: keeps the latest VDA 5050 `state` and `connection` of
//! every AGV at the configured sites in the module's tables.

mod convert;
#[allow(clippy::all)]
mod module_bindings;
mod sink;
mod writer;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ingest_core::config::{self, MqttConfig, Site, TelemetryConfig};
use ingest_core::health::Health;
use ingest_core::mqtt::Source;
use ingest_core::telemetry::{self, Telemetry};
use ingest_core::topic::Router;
use ingest_core::vda::Message;
use serde::Deserialize;

use crate::writer::{KINDS, SpacetimeConfig, Writer};

const SERVICE: &str = "spacetime-ingest";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    mqtt: MqttConfig,
    sites: Vec<Site>,
    spacetime: SpacetimeConfig,
    telemetry: TelemetryConfig,
    #[serde(default = "default_shutdown_flush_secs")]
    shutdown_flush_secs: u64,
}

fn default_shutdown_flush_secs() -> u64 {
    10
}

impl Config {
    fn load() -> Result<Self> {
        let config: Config = config::load(&format!("/etc/{SERVICE}/config.json"))?;
        config.mqtt.validate()?;
        config::validate_sites(&config.sites)?;
        config.spacetime.validate()?;
        Ok(config)
    }
}

fn main() -> Result<()> {
    if let Some(result) =
        ingest_core::app::healthcheck_command(|| Ok(Config::load()?.telemetry.health_port))
    {
        return result;
    }
    let runtime = tokio::runtime::Runtime::new().context("starting the runtime")?;
    runtime.block_on(run())
}

async fn run() -> Result<()> {
    telemetry::init_logging();
    let config = Config::load()
        .inspect_err(|error| tracing::error!(error = %format!("{error:#}"), "invalid config"))?;
    let instance = telemetry::instance_name();
    let telemetry = Telemetry::init(SERVICE, &instance, &config.telemetry)?;
    let metrics = telemetry.metrics.clone();
    tracing::info!(instance, sites = ?config.sites.iter().map(|s| &s.id).collect::<Vec<_>>(), "starting {SERVICE}");

    let health = Health::default();
    let writer = Arc::new(Writer::new(
        config.spacetime,
        metrics.clone(),
        health.component("spacetimedb"),
        Duration::from_secs(config.shutdown_flush_secs),
    ));
    let source = Source::new(
        config.mqtt,
        Arc::new(Router::new(config.sites)),
        KINDS.to_vec(),
        &instance,
    );

    ingest_core::app::run(
        health,
        config.telemetry.health_port,
        metrics,
        source,
        {
            let writer = writer.clone();
            move |message: Message| writer.queue_message(message)
        },
        {
            let writer = writer.clone();
            move |token| {
                let writer = writer.clone();
                async move { writer.run(token).await }
            }
        },
    )
    .await;
    writer.log_unwritten();
    drop(telemetry);
    Ok(())
}
