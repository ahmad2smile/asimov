//! Queues MQTT messages, each as a `Row` of the `INSERT`, and writes them to
//! TDengine. Every message kind has its own queue and its own connection, so
//! a slow kind never holds back the others. Every `batchMaxWaitMs`, or sooner
//! when a full statement is waiting, a queue goes out as one statement.
//!
//! Each queue is bounded: when full, the oldest message is dropped (newest
//! data matters most after an outage).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use ingest_core::health::Component;
use ingest_core::telemetry::Metrics;
use ingest_core::topic::Kind;
use ingest_core::vda::Message;
use serde::Deserialize;
use taos::Dsn;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::sink::{Sink, TIMEOUT};
use crate::sql::{self, Row};

/// TDengine refuses longer statements (1 MiB).
const MAX_SQL_BYTES: usize = 900_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TdengineConfig {
    /// taosAdapter WebSocket, e.g. `ws://tdengine:6041`
    dsn: String,
    database: String,
    #[serde(default = "default_user")]
    user: String,
    /// Overridden by the `TDENGINE_PASSWORD` environment variable.
    #[serde(default = "default_password")]
    password: String,
    #[serde(default = "default_batch_max_wait_ms")]
    batch_max_wait_ms: u64,
}

fn default_user() -> String {
    "root".into()
}

fn default_password() -> String {
    "taosdata".into()
}

fn default_batch_max_wait_ms() -> u64 {
    500
}

impl TdengineConfig {
    pub fn validate(&self) -> Result<()> {
        let database = &self.database;
        // Goes into SQL as a name.
        anyhow::ensure!(
            !database.is_empty()
                && database
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "tdengine.database {database:?}: only a-z, 0-9 and _"
        );
        Ok(())
    }

    fn dsn(&self) -> Result<Dsn> {
        let mut dsn: Dsn = self
            .dsn
            .parse()
            .with_context(|| format!("tdengine.dsn {:?}", self.dsn))?;
        dsn.username = Some(self.user.clone());
        dsn.password =
            Some(std::env::var("TDENGINE_PASSWORD").unwrap_or_else(|_| self.password.clone()));
        Ok(dsn)
    }
}

struct Entry {
    message: Message,
    row: Row,
}

/// The queue of one message kind.
struct Lane {
    kind: Kind,
    /// Oldest first.
    queue: Mutex<VecDeque<Entry>>,
    queued: Notify,
}

pub struct Writer {
    config: TdengineConfig,
    metrics: Metrics,
    ready: Component,
    shutdown_flush: Duration,
    /// Per kind.
    max_messages: usize,
    lanes: [Lane; 3],
}

impl Writer {
    pub fn new(
        config: TdengineConfig,
        max_messages: usize,
        metrics: Metrics,
        ready: Component,
        shutdown_flush: Duration,
    ) -> Self {
        Self {
            config,
            metrics,
            ready,
            shutdown_flush,
            max_messages: max_messages.max(1),
            lanes: Kind::ALL.map(|kind| Lane {
                kind,
                queue: Mutex::default(),
                queued: Notify::new(),
            }),
        }
    }

    fn lane(&self, kind: Kind) -> &Lane {
        self.lanes.iter().find(|lane| lane.kind == kind).unwrap()
    }

    /// Messages waiting in all queues.
    fn depth(&self) -> u64 {
        self.lanes
            .iter()
            .map(|lane| lane.queue.lock().unwrap().len() as u64)
            .sum()
    }

    /// Queues a message; drops the oldest one of its kind if that queue is full.
    pub fn queue_message(&self, message: Message) {
        let row = sql::row(&self.config.database, &message);
        let lane = self.lane(message.kind());
        let mut queue = lane.queue.lock().unwrap();
        let dropped = if queue.len() >= self.max_messages {
            queue.pop_front()
        } else {
            None
        };
        queue.push_back(Entry { message, row });
        drop(queue);
        self.metrics.buffer_depth(self.depth());
        lane.queued.notify_one();
        if let Some(dropped) = dropped {
            self.dropped(&dropped, "queue full");
        }
    }

    /// Logs every message still queued; for shutdown.
    pub fn log_unwritten(&self) {
        for lane in &self.lanes {
            for entry in lane.queue.lock().unwrap().drain(..) {
                not_written(&entry, "shutting down");
            }
        }
    }

    /// One connection per kind: connect, create the tables, then write every
    /// queue until shutdown or a lost connection (the supervisor reconnects).
    pub async fn run(&self, shutdown: CancellationToken) -> Result<()> {
        let result = self.connected(shutdown).await;
        self.ready.set(false);
        self.metrics.sink_connected(false);
        result
    }

    async fn connected(&self, shutdown: CancellationToken) -> Result<()> {
        let connect = async {
            let mut sinks = Vec::new();
            for _ in &self.lanes {
                sinks.push(Sink::connect(self.config.dsn()?).await?);
            }
            for sql in sql::create_sql(&self.config.database) {
                sinks[0].write(&sql).await?;
            }
            anyhow::Ok(sinks)
        };
        let sinks = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            sinks = tokio::time::timeout(TIMEOUT, connect) => sinks.context("connecting to TDengine timed out")??,
        };
        tracing::info!(dsn = %self.config.dsn, database = %self.config.database, "TDengine connected; tables ready");
        self.ready.set(true);
        self.metrics.sink_connected(true);

        // A failed lane stops the others through `stop`, but only between
        // writes: a write already sent is never cancelled, so no batch is lost.
        let stop = shutdown.child_token();
        let [a, b, c] = &self.lanes;
        let [sa, sb, sc] = &sinks[..] else {
            unreachable!("one sink per lane")
        };
        let results = tokio::join!(
            self.run_lane(a, sa, &stop, &shutdown),
            self.run_lane(b, sb, &stop, &shutdown),
            self.run_lane(c, sc, &stop, &shutdown),
        );
        results.0.and(results.1).and(results.2)
    }

    async fn run_lane(
        &self,
        lane: &Lane,
        sink: &Sink,
        stop: &CancellationToken,
        shutdown: &CancellationToken,
    ) -> Result<()> {
        let result = self.write_lane(lane, sink, stop, shutdown).await;
        if result.is_err() {
            stop.cancel();
        }
        result
    }

    async fn write_lane(
        &self,
        lane: &Lane,
        sink: &Sink,
        stop: &CancellationToken,
        shutdown: &CancellationToken,
    ) -> Result<()> {
        let max_wait = Duration::from_millis(self.config.batch_max_wait_ms);
        loop {
            let deadline = Instant::now() + max_wait;
            while !full_statement_waiting(lane) {
                tokio::select! {
                    _ = stop.cancelled() => {
                        return if shutdown.is_cancelled() { self.drain(lane, sink).await } else { Ok(()) };
                    }
                    _ = lane.queued.notified() => {}
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
            self.write_batch(lane, sink).await?;
        }
    }

    /// Shutdown: MQTT has stopped, so write what is left before the deadline.
    /// What is still left after it gets logged by `log_unwritten`.
    async fn drain(&self, lane: &Lane, sink: &Sink) -> Result<()> {
        let deadline = Instant::now() + self.shutdown_flush;
        while !lane.queue.lock().unwrap().is_empty() && Instant::now() < deadline {
            self.write_batch(lane, sink).await?;
        }
        Ok(())
    }

    /// Writes the oldest queued messages as one statement. On a lost
    /// connection they go back to the queue; messages TDengine rejects are
    /// logged and dropped.
    async fn write_batch(&self, lane: &Lane, sink: &Sink) -> Result<()> {
        let batch = take(lane, MAX_SQL_BYTES);
        if batch.is_empty() {
            return Ok(());
        }
        let started = Instant::now();
        let Err(error) = sink
            .insert(&sql::insert_sql(batch.iter().map(|e| &e.row)))
            .await
        else {
            self.written(lane, &batch, started.elapsed());
            return Ok(());
        };
        if !sink.alive().await {
            self.put_back(lane, batch);
            return Err(error.context("TDengine stopped answering"));
        }
        // A bad message fails the whole statement; find it by writing one by one.
        tracing::warn!(error = %format!("{error:#}"), messages = batch.len(), "TDengine rejected a batch; writing one by one");

        let mut rest = batch.into_iter();
        while let Some(entry) = rest.next() {
            let entry = [entry];
            let started = Instant::now();

            let Err(error) = sink
                .insert(&sql::insert_sql(entry.iter().map(|e| &e.row)))
                .await
            else {
                self.written(lane, &entry, started.elapsed());
                continue;
            };

            if !sink.alive().await {
                self.put_back(lane, entry.into_iter().chain(rest).collect());
                return Err(error.context("TDengine stopped answering"));
            }

            let message = &entry[0].message;
            self.metrics
                .sink_written(&message.agv.site, message.kind(), "dropped", 1);
            not_written(&entry[0], &format!("{error:#}"));
        }
        Ok(())
    }

    /// Puts messages back in front, after a failed write; drops the oldest
    /// ones that no longer fit.
    fn put_back(&self, lane: &Lane, entries: Vec<Entry>) {
        let mut queue = lane.queue.lock().unwrap();
        for entry in entries.into_iter().rev() {
            queue.push_front(entry);
        }
        let excess = queue.len().saturating_sub(self.max_messages);
        let dropped: Vec<Entry> = queue.drain(..excess).collect();
        drop(queue);
        for entry in &dropped {
            self.dropped(entry, "queue full");
        }
        self.metrics.buffer_depth(self.depth());
    }

    fn written(&self, lane: &Lane, entries: &[Entry], took: Duration) {
        for Entry { message, .. } in entries {
            self.metrics
                .sink_written(&message.agv.site, lane.kind, "ok", 1);
            self.metrics
                .latency(&message.agv.site, lane.kind, message.sent_at);
        }
        self.metrics.batch_size(lane.kind, entries.len() as u64);
        self.metrics.write_duration(lane.kind, took);
        self.metrics.buffer_depth(self.depth());
        tracing::debug!(
            stable = sql::stable(lane.kind),
            written = entries.len(),
            "batch written"
        );
    }

    fn dropped(&self, entry: &Entry, reason: &str) {
        self.metrics
            .buffer_dropped(&entry.message.agv.site, entry.message.kind());
        not_written(entry, reason);
    }
}

/// Stops counting at `MAX_SQL_BYTES`, so a long queue stays cheap.
fn full_statement_waiting(lane: &Lane) -> bool {
    let mut bytes = 0;
    lane.queue.lock().unwrap().iter().any(|entry| {
        bytes += entry.row.size();
        bytes >= MAX_SQL_BYTES
    })
}

/// Removes the oldest messages whose rows fit in `max_bytes` (at least one).
fn take(lane: &Lane, max_bytes: usize) -> Vec<Entry> {
    let mut queue = lane.queue.lock().unwrap();
    let mut bytes = 0;
    let count = queue
        .iter()
        .take_while(|entry| {
            bytes += entry.row.size();
            bytes <= max_bytes
        })
        .count()
        .max(1)
        .min(queue.len());
    queue.drain(..count).collect()
}

/// Logs a message that will not reach TDengine, with all its data.
fn not_written(entry: &Entry, reason: &str) {
    let message = &entry.message;
    tracing::warn!(
        agv = %message.agv,
        kind = %message.kind(),
        sent_at = message.sent_at,
        data = ?message.body,
        reason,
        "message not written to TDengine"
    );
}

#[cfg(test)]
mod tests {
    use ingest_core::health::Health;
    use ingest_core::topic::AgvKey;
    use ingest_core::vda::{Body, Connection, ConnectionState, Position, Visualization};

    use super::*;

    fn writer(max_messages: usize) -> Writer {
        let config = TdengineConfig {
            dsn: String::new(),
            database: "db".into(),
            user: default_user(),
            password: default_password(),
            batch_max_wait_ms: default_batch_max_wait_ms(),
        };
        let ready = Health::default().component("tdengine");
        Writer::new(
            config,
            max_messages,
            Metrics::disabled(),
            ready,
            Duration::ZERO,
        )
    }

    /// All messages have SQL parts of the same length; `serial` tells them apart.
    fn message(serial: &str) -> Message {
        let agv = AgvKey {
            site: "s".into(),
            manufacturer: "m".into(),
            serial_number: serial.into(),
        };
        let body = Body::Connection(Connection {
            header_id: 0,
            connection_state: ConnectionState::Online,
        });
        Message {
            agv,
            sent_at: 0,
            body,
        }
    }

    fn serials(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .map(|e| e.message.agv.serial_number.as_str())
            .collect()
    }

    fn connection(writer: &Writer) -> &Lane {
        writer.lane(Kind::Connection)
    }

    /// The gauge must follow queued messages, not only finished writes.
    #[test]
    fn reports_depth_when_messages_are_queued() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        let config = TdengineConfig {
            dsn: String::new(),
            database: "db".into(),
            user: default_user(),
            password: default_password(),
            batch_max_wait_ms: default_batch_max_wait_ms(),
        };
        let ready = Health::default().component("tdengine");
        let metrics = Metrics::new(&provider.meter("test"));
        let writer = Writer::new(config, 10, metrics, ready, Duration::ZERO);

        for serial in ["a", "b", "c"] {
            writer.queue_message(message(serial));
        }
        provider.force_flush().unwrap();

        let depth = exporter
            .get_finished_metrics()
            .unwrap()
            .iter()
            .flat_map(|r| r.scope_metrics())
            .flat_map(|s| s.metrics())
            .find(|m| m.name() == "ingest_buffer_depth")
            .map(|m| match m.data() {
                AggregatedMetrics::U64(MetricData::Gauge(g)) => {
                    g.data_points().next().unwrap().value()
                }
                _ => panic!("not a u64 gauge"),
            });
        assert_eq!(depth, Some(3));
    }

    #[test]
    fn takes_the_oldest_that_fit() {
        let writer = writer(10);
        for serial in ["a", "b", "c"] {
            writer.queue_message(message(serial));
        }
        let lane = connection(&writer);
        let size = lane.queue.lock().unwrap()[0].row.size();
        assert_eq!(serials(&take(lane, 2 * size + 1)), ["a", "b"]);
        assert_eq!(
            serials(&take(lane, 1)),
            ["c"],
            "one too big message still goes out"
        );
        assert!(take(lane, usize::MAX).is_empty());
    }

    #[test]
    fn drops_the_oldest_when_full() {
        let writer = writer(2);
        for serial in ["a", "b", "c"] {
            writer.queue_message(message(serial));
        }
        assert_eq!(serials(&take(connection(&writer), usize::MAX)), ["b", "c"]);
    }

    #[test]
    fn put_back_keeps_order_and_drops_the_oldest_beyond_max() {
        let writer = writer(3);
        let lane = connection(&writer);
        writer.queue_message(message("a"));
        writer.queue_message(message("b"));
        let taken = take(lane, usize::MAX);
        writer.queue_message(message("c"));
        writer.queue_message(message("d"));
        writer.put_back(lane, taken);
        assert_eq!(serials(&take(lane, usize::MAX)), ["b", "c", "d"]);
    }

    #[test]
    fn full_statement_waiting_counts_row_bytes() {
        let writer = writer(usize::MAX);
        let per_message = sql::row("db", &message("a")).size();
        for _ in 0..(MAX_SQL_BYTES - 1) / per_message {
            writer.queue_message(message("a"));
        }
        assert!(!full_statement_waiting(connection(&writer)));
        writer.queue_message(message("a"));
        assert!(full_statement_waiting(connection(&writer)));
    }

    #[test]
    fn every_kind_has_its_own_queue() {
        let writer = writer(1);
        let mut visualization = message("v");
        visualization.body = Body::Visualization(Visualization {
            agv_position: Position {
                x: 0.0,
                y: 0.0,
                theta: 0.0,
                map_id: "m".into(),
                localization_score: None,
            },
            velocity: None,
        });
        writer.queue_message(message("c"));
        writer.queue_message(visualization);

        let len = |kind| writer.lane(kind).queue.lock().unwrap().len();
        assert_eq!(len(Kind::Connection), 1);
        assert_eq!(
            len(Kind::Visualization),
            1,
            "no drop: another kind is full, not this one"
        );
        assert_eq!(len(Kind::State), 0);
        assert_eq!(writer.depth(), 2);
    }
}
