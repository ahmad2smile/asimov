# spacetime-ingest

MQTT -> SpacetimeDB. Keeps the latest VDA 5050 `state` and `connection` of every AGV at the configured sites, through the module's `ingest` reducer, which registers maps and AGVs itself (see [backend](../../backend/README.md)).

Runs as the `spacetime-ingest` compose service. Config: [`config/services/spacetime-ingest.json`](../../config/services/spacetime-ingest.json).

| Field | Default | |
| --- | --- | --- |
| `mqtt.url` | | `mqtt://host:port` |
| `mqtt.clientIdPrefix` | | Client id is `<prefix>-<hostname>` |
| `mqtt.group` | | Shared subscription group; instances with the same group split the load |
| `mqtt.keepAliveSecs` / `sessionExpirySecs` | `15` / `3600` | |
| `sites[]` | | `{ "id", "topicPrefix" }`, e.g. `hamburg`, `hamburg/uagv/v2` |
| `spacetime.uri` / `database` | | `ws://spacetimedb:3000` / `asimov` |
| `telemetry.otlpEndpoint` | off | OTLP gRPC for metrics |
| `telemetry.healthPort` | `8080` | `/healthz`, `/readyz` |
| `shutdownFlushSecs` | `10` | Time to write what is left on SIGTERM |

## Flow

```
MQTT (auto ack)   state | connection
Buffer (buffer.rs) latest message per AGV and kind; older -> dropped (stale_skipped)
Writer (writer.rs) 50 ms after new messages: one ingest(messages) call
SpacetimeDB       module registers maps and AGVs, stores the latest state and connection
```

| Result | Action |
| --- | --- |
| Written | Done |
| No answer for 30 s, or connection lost | Back to the buffer (newer ones win); reconnect with backoff |
| Module rejects the call | Sent one by one; rejected messages logged with their data and dropped |
| SIGTERM | MQTT stops, write what is left within `shutdownFlushSecs`, log the rest |

Env: `CONFIG_PATH` (default `/etc/spacetime-ingest/config.json`), `RUST_LOG`.

After a module change: `npm run spacetime:generate:rust` (regenerates `src/module_bindings/`, never edit by hand).
