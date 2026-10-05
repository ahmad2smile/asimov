# AGV fleet simulator

Rust service: 5 warehouse sites with 2–5 AGVs each, from real VDA 5050 manufacturers
(KUKA, Jungheinrich, Linde, STILL, AGILOX, MiR, ek robotics), publishing
[VDA 5050 2.1.0](https://github.com/VDA5050/VDA5050/blob/release/2.1.0/VDA5050_EN.md)
over MQTT. Runs as the `agv-simulator` compose service; the fleet is listed
in its startup log.

## Topics

`<siteId>/uagv/v2/<manufacturer>/<serialNumber>/…` (site prefix: see [ADR-0001](../../docs/adr/0001-mqtt-ingest-services.md))

| Topic | QoS | Sent |
| --- | --- | --- |
| `connection` | 1, retained | `ONLINE` / `OFFLINE`; `CONNECTIONBROKEN` as last will |
| `state` | 0 | On events, at least every 30 s |
| `visualization` | 0 | Every second |

```bash
docker compose exec mosquitto mosquitto_sub -t '+/uagv/v2/+/+/state' -v
```

## Behaviour

Each robot loops: dock → `pick` pallet → rack → `drop`, and goes to its
charger (`startCharging`) below 25% battery. Each mission is a VDA 5050 order
with node, edge, and action states. Each site has its own map
(`hamburg-warehouse`, …). Robots dispatch their own orders; `order` and
`instantActions` are not handled.

## Configuration

Set in a root `.env`, then `npm run infra:up`:

| Variable | Default |
| --- | --- |
| `SITE_COUNT` | `5` (max 8) |
| `ROBOTS_PER_SITE_MIN` / `_MAX` | `2` / `5` |
| `SIM_SEED` (same seed = same fleet) | `asimov` |
| `STATE_MAX_INTERVAL_MS` | `30000` |
| `VISUALIZATION_INTERVAL_MS` (`0` = off) | `1000` |
| `MQTT_URL` | `mqtt://localhost:1883` |

## Develop

```bash
npm run simulator:test                       # 4 simulated hours, checked against the official schemas in tests/
cargo run --release --bin asimov-simulator   # run locally (from services/); stop the compose service first
```
