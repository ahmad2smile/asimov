//! MQTT source: subscribes to the configured sites, decodes each message
//! and hands it to the service.
//!
//! - MQTT 5 persistent session (`clean_start = false` + session expiry), so
//!   QoS 1 messages wait on the broker while the service reconnects.
//! - Auto acks: a QoS 1 message is acked as soon as it arrives, so the
//!   broker never waits on a slow service.
//! - Reconnects forever with backoff; subscribes again after every connect
//!   (cheap, and picks up config changes).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::mqttbytes::v5::{Filter, Packet, Publish};
use rumqttc::v5::{AsyncClient, Event, EventLoop, MqttOptions};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::backoff::Backoff;
use crate::config::MqttConfig;
use crate::health::Component;
use crate::reject_log::RejectLog;
use crate::telemetry::Metrics;
use crate::topic::{Kind, Router};
use crate::vda::{self, Message};

/// VDA 5050 `state` messages can be large (many nodes, edges, actions).
const MAX_PACKET_SIZE: u32 = 1024 * 1024;
/// Requests (subscribes, disconnect) queued for the event loop.
const REQUEST_CAPACITY: usize = 4096;

pub struct Source {
    pub config: MqttConfig,
    pub router: Arc<Router>,
    /// Message kinds this service needs.
    pub kinds: Vec<Kind>,
    pub client_id: String,
}

impl Source {
    pub fn new(config: MqttConfig, router: Arc<Router>, kinds: Vec<Kind>, instance: &str) -> Self {
        let client_id = format!("{}-{instance}", config.client_id_prefix);
        Self { config, router, kinds, client_id }
    }

    fn options(&self) -> Result<MqttOptions> {
        let url = format!("{}?client_id={}", self.config.url, self.client_id);
        let mut options = MqttOptions::parse_url(url).with_context(|| format!("mqtt.url {}", self.config.url))?;
        options
            .set_keep_alive(Duration::from_secs(self.config.keep_alive_secs.into()))
            .set_clean_start(false)
            .set_session_expiry_interval(Some(self.config.session_expiry_secs))
            .set_max_packet_size(Some(MAX_PACKET_SIZE));
        Ok(options)
    }

    /// Runs until `shutdown`, reconnecting on every error. Each message that
    /// decodes is passed to `handle`.
    pub async fn run(
        &self,
        handle: impl Fn(Message) + Send + Sync,
        metrics: &Metrics,
        ready: &Component,
        shutdown: CancellationToken,
    ) -> Result<()> {
        let (client, mut events) = AsyncClient::new(self.options()?, REQUEST_CAPACITY);
        let mut backoff = Backoff::default();
        let mut rejections = RejectLog::default();
        let mut ever_connected = false;
        tracing::info!(client_id = %self.client_id, url = %self.config.url, "connecting to MQTT");

        loop {
            let event = tokio::select! {
                _ = shutdown.cancelled() => break,
                event = events.poll() => event,
            };
            match event {
                Ok(Event::Incoming(Packet::ConnAck(connack))) => {
                    backoff.reset();
                    if ever_connected {
                        metrics.reconnected("mqtt");
                    }
                    ever_connected = true;
                    tracing::info!(session_present = connack.session_present, "MQTT connected");
                    self.subscribe(&client)?;
                    ready.set(true);
                    metrics.mqtt_connected(true);
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    self.receive(&publish, &handle, metrics, &mut rejections);
                }
                Ok(Event::Incoming(Packet::Disconnect(disconnect))) => {
                    tracing::warn!(reason = ?disconnect.reason_code, "broker disconnected us");
                }
                Ok(_) => {}
                Err(error) => {
                    ready.set(false);
                    metrics.mqtt_connected(false);
                    let delay = backoff.next_delay();
                    tracing::warn!(error = %error, delay_ms = delay.as_millis() as u64, "MQTT connection lost; reconnecting");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
            }
        }

        ready.set(false);
        metrics.mqtt_connected(false);
        disconnect(&client, &mut events).await;
        Ok(())
    }

    fn subscribe(&self, client: &AsyncClient) -> Result<()> {
        let filters: Vec<Filter> = self
            .router
            .subscriptions(&self.config.group, &self.kinds)
            .into_iter()
            .map(|subscription| Filter::new(subscription.filter, QoS::AtLeastOnce))
            .collect();
        tracing::info!(filters = ?filters.iter().map(|f| &f.path).collect::<Vec<_>>(), "subscribing");
        // try_: this task drives the event loop, so it must not wait on the queue.
        client.try_subscribe_many(filters).context("subscribing")?;
        Ok(())
    }

    fn receive(&self, publish: &Publish, handle: &impl Fn(Message), metrics: &Metrics, rejections: &mut RejectLog) {
        let topic = String::from_utf8_lossy(&publish.topic);
        let route = match self.router.route(&topic) {
            Ok(route) => route,
            Err(error) => {
                metrics.rejected(None, None, error.reason());
                rejections.log(error.reason(), &topic);
                return;
            }
        };
        let site = &route.agv.site;
        metrics.received(site, route.kind);
        if !self.kinds.contains(&route.kind) {
            metrics.rejected(Some(site), Some(route.kind), "unused_kind");
            return;
        }
        match vda::decode(&route, &publish.payload) {
            Ok(message) => handle(message),
            Err(reject) => {
                metrics.rejected(Some(site), Some(route.kind), reject.reason());
                rejections.log(reject.reason(), &topic);
            }
        }
    }
}

/// Sends DISCONNECT and polls until it is out, so the broker does not
/// publish a will or wait for the keepalive.
async fn disconnect(client: &AsyncClient, events: &mut EventLoop) {
    if client.try_disconnect().is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.poll()).await {
        if matches!(event, Event::Outgoing(rumqttc::Outgoing::Disconnect)) {
            break;
        }
    }
}
