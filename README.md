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

## NOTE

### Done

Project only shows basic Backend + Ingestion focusing on ingestion throughput and realtime backend capability for frontend presentation.

### Planned

- Ingest visualization from TimeSeries for graph/simulation.
- Increase Backend Ingestion to something around ~300,000 transactions/sec (SpacetimeDb Bench Limits)

## Dashboard

<img width="1290" height="849" alt="Screenshot 2026-10-05 at 12 27 11" src="https://github.com/user-attachments/assets/7f08b9fe-e074-45e9-a7cd-34fcba23d182" />

## Observability

<img width="1457" height="856" alt="image" src="https://github.com/user-attachments/assets/f8c52b04-3eaf-42bd-8f1f-4806ba13f6eb" />

