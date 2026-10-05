# services

Rust workspace for the MQTT side. Design: [ADR-0001](../docs/adr/0001-mqtt-ingest-services.md).

| Crate | What |
| --- | --- |
| [`simulator/`](simulator/README.md) | Simulated VDA 5050 AGV fleet |
| [`spacetime-ingest/`](spacetime-ingest/README.md) | MQTT -> SpacetimeDB (latest state per AGV) |
| [`tdengine-ingest/`](tdengine-ingest/README.md) | MQTT -> TDengine (message history) |
| `ingest-core/` | Shared: config, MQTT source, VDA 5050 decoding, telemetry, health, supervisor |
| `ingest-e2e/` | End-to-end tests against the compose stack |

```bash
npm run services:test   # unit tests
npm run services:e2e    # end-to-end; needs `npm run infra:up`, restarts containers, ~2 min
```

Docker images build with `services/` as context: `docker compose build <service>`.
