# asimov

SpacetimeDB app (TypeScript backend, React frontend) with local TDengine, Mosquitto,
and simulated VDA 5050 robots.

| Path | What |
| --- | --- |
| `backend/` | SpacetimeDB module in TypeScript - see [backend/README.md](backend/README.md) |
| `src/` | React + Vite fleet dashboard (TypeScript, Tailwind, [shadcn/ui](https://ui.shadcn.com) in `src/components/ui`) |
| `simulator/` | AGV fleet simulator in Rust – see [simulator/README.md](simulator/README.md) |
| `compose.yaml` | Local services |
| `config/` | Tool configs (Mosquitto, Vite) |

Needs Node.js 22.18+, Rust (simulator only), Docker, and the [SpacetimeDB CLI](https://spacetimedb.com/install) 2.10.x.

## Run

```bash
npm install
npm run infra:up            # start services
npm run spacetime:publish   # publish the module as `asimov`
npm run dev                 # http://localhost:5173
```

After changing the module: `npm run spacetime:publish`, then
`npm run spacetime:generate`. For an incompatible schema change, add
`--delete-data=on-conflict` (clears data).

- Module test: `npm run spacetime:test`. Dashboard E2E (Playwright): `PLAYWRIGHT_BROWSERS_PATH=0 npx playwright install chromium` once, then `npm run e2e`. Both need `infra:up`. Each E2E test saves a screenshot to `test-results/screenshots/<file> - <test>.png`.
- Seed one simulated fleet snapshot: `scripts/seed-spacetime.sh [database]`.

Add shadcn components with `npx shadcn@latest add <name>` (config in `components.json`).

## Services

| Service | Address |
| --- | --- |
| SpacetimeDB | `http://127.0.0.1:3000` |
| TDengine REST / Explorer | `http://127.0.0.1:6041` / `:6060` (`root` / `taosdata`) |
| Mosquitto | `mqtt://127.0.0.1:1883`, `ws://127.0.0.1:9001` |
| AGV simulator | publishes to `uagv/v2/#` on Mosquitto |

`npm run infra:logs` follows logs, `npm run infra:down` stops (data kept),
`docker compose down -v` deletes all data.

- AGV data: SpacetimeDB `agv_state` (latest state and connection per AGV; the dashboard pages via the `agv_page` procedure and counts via the `fleet_stats` view), TDengine `asimov.agv_visualization` (schema in `config/tdengine.sql`).
- TDengine's native port 6030 needs `127.0.0.1 tdengine` in `/etc/hosts`.
- Keep the SpacetimeDB image, CLI, `spacetimedb` crate, and npm package on the same version.
