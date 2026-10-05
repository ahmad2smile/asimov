//! Keeps the latest message per AGV and kind, and writes them to SpacetimeDB.
//! Shortly after new messages arrive, all waiting messages go out in one
//! `ingest` reducer call; the module registers maps and AGVs itself.
//!
//! SpacetimeDB keeps only the latest of each per AGV, so an older message
//! still waiting here is replaced. The waiting set is bounded by the number
//! of AGVs, so a long outage cannot use up memory.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ingest_core::health::Component;
use ingest_core::telemetry::Metrics;
use ingest_core::topic::{AgvKey, Kind};
use ingest_core::vda::Message;
use serde::Deserialize;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::sink::{Sink, TIMEOUT};

pub const KINDS: [Kind; 2] = [Kind::State, Kind::Connection];

/// Collects messages for this long after a wake-up, so a burst becomes one call.
const DEBOUNCE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpacetimeConfig {
    /// e.g. `ws://spacetimedb:3000`
    uri: String,
    database: String,
}

impl SpacetimeConfig {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.uri.starts_with("ws://") || self.uri.starts_with("wss://"),
            "spacetime.uri must start with ws:// or wss://"
        );
        anyhow::ensure!(
            !self.database.is_empty(),
            "spacetime.database must not be empty"
        );
        Ok(())
    }
}

pub struct Writer {
    config: SpacetimeConfig,
    metrics: Metrics,
    ready: Component,
    shutdown_flush: Duration,
    waiting: Mutex<HashMap<(AgvKey, Kind), Message>>,
    changed: Notify,
}

impl Writer {
    pub fn new(
        config: SpacetimeConfig,
        metrics: Metrics,
        ready: Component,
        shutdown_flush: Duration,
    ) -> Self {
        Self {
            config,
            metrics,
            ready,
            shutdown_flush,
            waiting: Mutex::default(),
            changed: Notify::new(),
        }
    }

    /// Buffers a message; an older one of the same AGV and kind is skipped.
    pub fn queue_message(&self, message: Message) {
        let (site, kind) = (message.agv.site.clone(), message.kind());
        if !self.offer(message) {
            self.metrics.stale_skipped(&site, kind);
        }
    }

    /// Keeps the message unless a newer one of the same AGV and kind is
    /// waiting; then returns false. Equal times: the new one wins (a
    /// redelivered duplicate carries the same data).
    fn offer(&self, message: Message) -> bool {
        let mut waiting = self.waiting.lock().unwrap();
        let key = (message.agv.clone(), message.kind());
        if waiting
            .get(&key)
            .is_some_and(|older| older.sent_at > message.sent_at)
        {
            return false;
        }
        waiting.insert(key, message);
        drop(waiting);
        self.changed.notify_one();
        true
    }

    /// All waiting messages, oldest first.
    fn take_all(&self) -> Vec<Message> {
        let mut all: Vec<_> = self
            .waiting
            .lock()
            .unwrap()
            .drain()
            .map(|(_, message)| message)
            .collect();
        all.sort_by_key(|message| message.sent_at);
        all
    }

    fn depth(&self) -> u64 {
        self.waiting.lock().unwrap().len() as u64
    }

    /// Logs every message still buffered; for shutdown.
    pub fn log_unwritten(&self) {
        for message in self.take_all() {
            not_written(&message, "shutting down");
        }
    }

    /// One connection: connect, then write the buffer until shutdown or a
    /// lost connection (the supervisor reconnects).
    pub async fn run(&self, shutdown: CancellationToken) -> Result<()> {
        let result = self.connected(shutdown).await;
        self.ready.set(false);
        self.metrics.sink_connected(false);
        result
    }

    async fn connected(&self, shutdown: CancellationToken) -> Result<()> {
        let connect = Sink::connect(&self.config.uri, &self.config.database);
        let sink = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            sink = tokio::time::timeout(TIMEOUT, connect) => sink.context("connecting to SpacetimeDB timed out")??,
        };
        tracing::info!(uri = %self.config.uri, database = %self.config.database, "SpacetimeDB connected");
        self.ready.set(true);
        self.metrics.sink_connected(true);

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return self.drain(&sink).await,
                reason = sink.closed() => bail!("SpacetimeDB {reason}"),
                _ = self.changed.notified() => {}
            }
            tokio::time::sleep(DEBOUNCE).await;
            self.write_batch(&sink, TIMEOUT).await?;
        }
    }

    /// Shutdown: MQTT has stopped, so write what is left within the deadline.
    /// What is still left after it gets logged by `log_unwritten`.
    async fn drain(&self, sink: &Sink) -> Result<()> {
        self.write_batch(sink, self.shutdown_flush).await
    }

    /// Writes all waiting messages in one call. On a lost connection they go
    /// back to the buffer; messages the module rejects are logged and dropped.
    async fn write_batch(&self, sink: &Sink, limit: Duration) -> Result<()> {
        let batch = self.take_all();
        if batch.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        let rejected = match sink.write(&batch, limit).await {
            Ok(Ok(())) => {
                self.written(&batch, started.elapsed());
                return Ok(());
            }
            Ok(Err(rejected)) => rejected,
            Err(error) => {
                self.put_back(batch);
                return Err(error);
            }
        };
        // A bad message fails the whole call; find it by writing one by one.
        tracing::warn!(error = %rejected, messages = batch.len(), "SpacetimeDB rejected a batch; writing one by one");

        let mut rest = batch.into_iter();
        while let Some(message) = rest.next() {
            let one = [message];
            let started = Instant::now();

            match sink.write(&one, limit).await {
                Ok(Ok(())) => self.written(&one, started.elapsed()),
                Ok(Err(error)) => {
                    let [message] = &one;
                    self.metrics
                        .sink_written(&message.agv.site, message.kind(), "dropped", 1);
                    not_written(message, &error);
                }
                Err(error) => {
                    self.put_back(one.into_iter().chain(rest).collect());
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn written(&self, messages: &[Message], took: Duration) {
        for message in messages {
            self.metrics
                .sink_written(&message.agv.site, message.kind(), "ok", 1);
            self.metrics
                .latency(&message.agv.site, message.kind(), message.sent_at);
        }
        for kind in KINDS {
            let written = messages.iter().filter(|m| m.kind() == kind).count();
            if written > 0 {
                self.metrics.batch_size(kind, written as u64);
                self.metrics.write_duration(kind, took);
                tracing::debug!(%kind, written, "batch written");
            }
        }
        self.metrics.buffer_depth(self.depth());
    }

    /// Newer messages that arrived meanwhile win.
    fn put_back(&self, messages: Vec<Message>) {
        for message in messages {
            self.offer(message);
        }
        self.metrics.buffer_depth(self.depth());
    }
}

/// Logs a message that will not reach SpacetimeDB, with all its data.
fn not_written(message: &Message, reason: &str) {
    tracing::warn!(
        agv = %message.agv,
        kind = %message.kind(),
        sent_at = message.sent_at,
        data = ?message.body,
        reason,
        "message not written to SpacetimeDB"
    );
}

#[cfg(test)]
mod tests {
    use ingest_core::health::Health;
    use ingest_core::vda::{Body, Connection, ConnectionState};

    use super::*;

    fn writer() -> Writer {
        let config = SpacetimeConfig {
            uri: "ws://localhost:3000".into(),
            database: "db".into(),
        };
        let ready = Health::default().component("spacetime");
        Writer::new(config, Metrics::disabled(), ready, Duration::ZERO)
    }

    fn connection(serial: &str, sent_at: i64, connection_state: ConnectionState) -> Message {
        let agv = AgvKey {
            site: "s".into(),
            manufacturer: "m".into(),
            serial_number: serial.into(),
        };
        Message {
            agv,
            sent_at,
            body: Body::Connection(Connection {
                header_id: 0,
                connection_state,
            }),
        }
    }

    #[test]
    fn keeps_the_newest_message_per_agv_and_kind() {
        let writer = writer();
        assert!(writer.offer(connection("1", 20, ConnectionState::Online)));
        assert!(
            !writer.offer(connection("1", 10, ConnectionState::Offline)),
            "older is stale"
        );
        assert!(
            writer.offer(connection("1", 20, ConnectionState::Connectionbroken)),
            "same time wins"
        );
        assert!(writer.offer(connection("2", 5, ConnectionState::Online)));
        assert_eq!(writer.depth(), 2);

        let all = writer.take_all();
        assert_eq!(
            all.iter().map(|m| m.sent_at).collect::<Vec<_>>(),
            [5, 20],
            "oldest first"
        );
        assert_eq!(
            all[1].body,
            Body::Connection(Connection {
                header_id: 0,
                connection_state: ConnectionState::Connectionbroken
            })
        );
        assert_eq!(writer.depth(), 0);
    }

    #[test]
    fn putting_taken_messages_back_keeps_newer_ones() {
        let writer = writer();
        writer.offer(connection("1", 10, ConnectionState::Online));
        let taken = writer.take_all();
        writer.offer(connection("1", 30, ConnectionState::Offline));
        writer.put_back(taken);
        assert_eq!(writer.take_all()[0].sent_at, 30);
    }
}
