//! Process skeleton shared by the ingest services: health server, supervised
//! MQTT source and sink, and ordered shutdown (stop MQTT first, then let the
//! sink write what is left).

use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::health::{self, Health};
use crate::mqtt::Source;
use crate::supervisor::supervise;
use crate::telemetry::Metrics;
use crate::vda::Message;

pub async fn run<H, S, Fut>(health: Health, health_port: u16, metrics: Metrics, source: Source, handle: H, sink: S)
where
    H: Fn(Message) + Send + Sync + 'static,
    S: FnMut(CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let health_stop = CancellationToken::new();
    let health_server = tokio::spawn(supervise("health", health_stop.clone(), metrics.clone(), {
        let health = health.clone();
        move |token| health::serve(health_port, health.clone(), token)
    }));

    let sink_stop = CancellationToken::new();
    let sink_task = tokio::spawn(supervise("sink", sink_stop.clone(), metrics.clone(), sink));

    let mqtt_stop = CancellationToken::new();
    let source = Arc::new(source);
    let handle = Arc::new(handle);
    let mqtt_ready = health.component("mqtt");
    let mqtt_task = tokio::spawn(supervise("mqtt", mqtt_stop.clone(), metrics.clone(), {
        let metrics = metrics.clone();
        move |token| {
            let (source, handle, metrics, ready) = (source.clone(), handle.clone(), metrics.clone(), mqtt_ready.clone());
            async move { source.run(|message| handle(message), &metrics, &ready, token).await }
        }
    }));

    crate::wait_for_shutdown_signal().await;
    tracing::info!("shutting down");
    mqtt_stop.cancel();
    let _ = mqtt_task.await;
    sink_stop.cancel();
    let _ = sink_task.await;
    health_stop.cancel();
    let _ = health_server.await;
    tracing::info!("stopped");
}

/// `<binary> healthcheck [path]`: exits 0 when the local health endpoint
/// answers 200. Returns None when the binary was not called that way.
pub fn healthcheck_command(port: impl FnOnce() -> Result<u16>) -> Option<Result<()>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("healthcheck") {
        return None;
    }
    let path = args.next().unwrap_or_else(|| "/healthz".into());
    Some(port().and_then(|port| health::healthcheck(port, &path)))
}
