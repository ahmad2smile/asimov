# tdengine-ingest

MQTT -> TDengine. Stores every VDA 5050 `state`, `visualization` and `connection` message of the configured sites. Creates the database and super tables on connect (schema in [`src/sql.rs`](src/sql.rs)); subtables are created on their first insert.

Runs as the `tdengine-ingest` compose service. Config: [`config/services/tdengine-ingest.json`](../../config/services/tdengine-ingest.json). `mqtt`, `sites`, `telemetry`, `shutdownFlushSecs` as in [spacetime-ingest](../spacetime-ingest/README.md), plus:

| Field | Default | |
| --- | --- | --- |
| `tdengine.dsn` | | taosAdapter WebSocket, `ws://tdengine:6041` |
| `tdengine.database` | | Lowercase, digits, `_` |
| `tdengine.user` / `password` | `root` / `taosdata` | Env `TDENGINE_PASSWORD` overrides |
| `tdengine.batchMaxWaitMs` | `500` | |
| `buffer.maxMessages` | | Messages kept per kind while TDengine is away; oldest dropped beyond |

| Super table | Tags |
| --- | --- |
| `agv_visualization` | site, manufacturer, serial_number, map_id |
| `agv_state` | site, manufacturer, serial_number, map_id |
| `agv_connection` | site, manufacturer, serial_number |

## Flow

```
MQTT (auto ack)    message -> sql::row()
Queues (writer.rs) one per kind (state, visualization, connection); full -> oldest dropped and logged
Writers (writer.rs) one TDengine connection per kind; every batchMaxWaitMs (or 1 MB of SQL):
                   one INSERT INTO <subtable + tags> VALUES (..) (..) <subtable + tags> ...
TDengine           taosAdapter WebSocket
```

Rows of one subtable share one `USING ... TAGS` head, so the server parses the tags once per subtable. A failed connection restarts all three.

| Result | Action |
| --- | --- |
| Written | Removed from the buffer |
| Server stops answering (10 s per call) | Back to the buffer; reconnect with backoff |
| Server rejects the batch | Written one by one; rejected messages logged with their data and dropped |
| SIGTERM | MQTT stops, write what is left for up to `shutdownFlushSecs`, log the rest |
