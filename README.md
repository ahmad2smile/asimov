# asimov

SpacetimeDB app (TypeScript backend, React frontend) with local TDengine, Mosquitto,
simulated VDA 5050 robots, and MQTT ingest services.

| Path | What |
| --- | --- |
| `backend/` | SpacetimeDB module in TypeScript - see [backend/README.md](backend/README.md) |
| `frontend/` | React + Vite fleet dashboard (TypeScript, Tailwind, [shadcn/ui](https://ui.shadcn.com) in `frontend/components/ui`) |
| `services/` | Rust: AGV simulator and MQTT ingest services - see [services/README.md](services/README.md) |
| `docs/adr/` | Architecture decisions |
| `compose.yaml` | Local services |
| `config/` | Tool and service configs |

Needs Node.js 22.18+, Rust, Docker, and the [SpacetimeDB CLI](https://spacetimedb.com/install) 2.10.x.

## Run

```bash
npm install
npm run infra:up            # build and start services (publishes the module as `asimov` first)
npm run dev                 # http://localhost:5173
```

After changing the module: `npm run spacetime:publish` (runs in a container, which owns the database), then `npm run spacetime:generate`
and `npm run spacetime:generate:rust`. For an incompatible schema change, add
`--delete-data=on-conflict` (clears data).

| Test | Command (needs `infra:up`) |
| --- | --- |
| Module | `npm run spacetime:test` |
| Dashboard E2E (Playwright) | `npm run e2e` (once: `PLAYWRIGHT_BROWSERS_PATH=0 npx playwright install chromium`); screenshots in `test-results/screenshots/` |
| Rust unit / simulator | `npm run services:test` / `npm run simulator:test` |
| Ingest E2E | `npm run services:e2e` (restarts containers) |

Seed one simulated fleet snapshot: `scripts/seed-spacetime.sh [database]`.
Add shadcn components with `npx shadcn@latest add <name>` (config in `components.json`).

## Services

| Service | Address |
| --- | --- |
| SpacetimeDB | `http://127.0.0.1:3000` |
| TDengine REST / Explorer | `http://127.0.0.1:6041` / `:6060` (`root` / `taosdata`) |
| Mosquitto | `mqtt://127.0.0.1:1883`, `ws://127.0.0.1:9001` |
| AGV simulator | publishes to `<site>/uagv/v2/#` on Mosquitto |
| spacetime-ingest / tdengine-ingest | MQTT -> SpacetimeDB `asimov` / TDengine `asimov` |
| Grafana (dashboard "MQTT ingest") / Prometheus | `http://127.0.0.1:3001` / `:9090` |

`npm run infra:logs` follows logs, `npm run infra:down` stops (data kept),
`docker compose down -v` deletes all data.

- AGV data: SpacetimeDB `agv_state` (latest state and connection per AGV), TDengine `agv_state`, `agv_visualization`, `agv_connection` (history; see [tdengine-ingest](services/tdengine-ingest/README.md)).
- TDengine's native port 6030 needs `127.0.0.1 tdengine` in `/etc/hosts`.
- Keep the SpacetimeDB image, CLI, `spacetimedb` npm package and `spacetimedb-sdk` crate on the same version.
