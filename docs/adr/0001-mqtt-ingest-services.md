# ADR-0001: MQTT ingest services

**Status:** Accepted - 2026-10-01

Two long-running Rust services move VDA 5050 messages from MQTT into storage:

| Service | Stores | Buffer |
| --- | --- | --- |
| `spacetime-ingest` | Latest `state` + `connection` per AGV (SpacetimeDB reducers) | In memory, newest message per AGV and kind |
| `tdengine-ingest` | History of `state`, `visualization`, `connection` (TDengine) | In memory, bounded FIFO, drops oldest when full |

## 1. Site is a topic prefix

`<siteId>/uagv/v2/<manufacturer>/<serialNumber>/<topic>`

- VDA 5050 2.1.0 (6.1) leaves the topic structure open and has no site concept.
- The broker filters by site, so a service only gets its sites' traffic.
- `connection` has no `mapId`; only the topic says where it belongs.
- Rejected: filter `uagv/v2/#` by payload `mapId`; static AGV lists in config.

## 2. Rust workspace in `services/`

`ingest-core` (shared), `spacetime-ingest`, `tdengine-ingest`, `simulator`, `ingest-e2e`. MQTT 5 via `rumqttc`, `spacetimedb-sdk` (same version as the server), `taos` over WebSocket (no native lib).

## 3. Several instances per service

| Topic | Subscription |
| --- | --- |
| `state`, `visualization` | `$share/<group>/...`: instances split the load |
| `connection` | Every instance: retained messages never reach shared subscriptions |

- One AGV's messages can reach different instances out of order, so sinks ignore older data:
  - SpacetimeDB reducers take the VDA header timestamp (`sentAt`) and skip writes older than the stored one.
  - TDengine overwrites the same subtable + timestamp.
- Client id `<clientIdPrefix>-<hostname>`; MQTT 5 session expiry clears sessions of removed instances.

## 4. Data rules

- Messages without a valid header `timestamp`, with a header AGV that does not match the topic, or that do not fit the schema are rejected (metric + rate-limited log).
- QoS 1 subscriptions and persistent sessions; acked as soon as the message arrives, so the broker never waits on a service.
- Buffers are in memory: buffered messages are lost if the process dies. VDA 5050 sends `state`/`visualization` with QoS 0, so messages sent while a service is away from the broker are lost by design.
- TDengine: `map_id` is a tag; an AGV on another map is another subtable. No position in `agv_state` (VDA 5050 sends it in `visualization`).
- TDengine super tables are created by `tdengine-ingest` on connect; subtables are created on their first insert.

## 5. Resilience

- Every task (MQTT, sink, health server) is restarted on error, panic, or return, with exponential backoff and jitter (100 ms to 30 s). Only an invalid config exits the process.
- Every TDengine call has a 10 s timeout; a stopped server can otherwise leave a call waiting forever.
- TDengine: all buffered messages go out as one multi-table `INSERT`. If the server stops answering, they stay buffered and the connection is retried. If it rejects the batch, the messages are written one by one; rejected ones are logged with their data and dropped.
- SpacetimeDB: all waiting messages go out in one `ingest` reducer call; the module registers maps and AGVs. No answer for 30 s, or a lost connection: the messages go back to the buffer (newer ones win) and the connection is retried. If the module rejects the call, the messages are sent one by one; rejected ones are logged with their data.
- SIGTERM: stop MQTT, then write what is left within `shutdownFlushSecs`.
- `/healthz` (alive) and `/readyz` (MQTT + sink connected); `<binary> healthcheck [path]` for container healthchecks.

## 6. Observability

Logs: JSON lines on stdout, level from `RUST_LOG`. Metrics: OTLP -> OTel Collector -> Prometheus -> Grafana (dashboard "MQTT ingest").

| Metric | Labels |
| --- | --- |
| `ingest_mqtt_messages_received_total` | site, kind |
| `ingest_mqtt_messages_rejected_total` | site, kind, reason |
| `ingest_mqtt_connected`, `ingest_sink_connected` | |
| `ingest_reconnects_total` | component |
| `ingest_sink_writes_total` | site, kind, result |
| `ingest_sink_stale_skipped_total` | site, kind |
| `ingest_sink_write_duration_seconds`, `ingest_sink_batch_size` | kind |
| `ingest_buffer_depth`, `ingest_buffer_dropped_total` | |
| `ingest_end_to_end_latency_seconds` (header timestamp to write) | site, kind |
| `ingest_task_restarts_total` | task |

Every metric also has `job` (service) and `instance` (hostname).
