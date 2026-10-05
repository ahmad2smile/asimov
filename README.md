# asimov

Live fleet dashboard for simulated VDA 5050 warehouse robots (AGVs).

Needs Node.js 22.18+, Rust, Docker, and the [SpacetimeDB CLI](https://spacetimedb.com/install) 2.10.x.

## Run

```bash
npm install
npm run infra:up    # build and start all services
npm run dev         # dashboard at http://localhost:5173
```

`npm run infra:down` stops everything (data kept).

## Services

| Service              | What it does                                                                                             |
| -------------------- | -------------------------------------------------------------------------------------------------------- |
| `frontend`           | React dashboard that shows the fleet live                                                                |
| `backend`            | SpacetimeDB module that stores the latest state and connection of each AGV ([README](backend/README.md)) |
| SpacetimeDB          | Database and server for the module (`http://127.0.0.1:3000`)                                             |
| AGV simulator        | Robots that publish VDA 5050 messages over MQTT                                                          |
| Mosquitto            | MQTT broker between the simulator and the ingest services                                                |
| spacetime-ingest     | Writes the latest robot data from MQTT into SpacetimeDB                                                  |
| tdengine-ingest      | Writes robot history from MQTT into TDengine                                                             |
| TDengine             | Time series database for robot history (`http://127.0.0.1:6041`)                                         |
| Prometheus / Grafana | Ingest metrics and dashboard (`http://127.0.0.1:9090` / `:3001`)                                         |

Supporting Services are described in [services/README.md](services/README.md).

# NOTE

## Done

Project only shows basic Backend + Ingestion focusing on ingestion throughput and realtime backend capability for frontend presentation.

## Planned

Ingest visualization from TimeSeries for graph/simulation.
