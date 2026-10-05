# TODOs

## Pending

- Tracing (spans) for the ingest services; logs and metrics are done
- No reducer deletes AGVs or maps, so stale rows (and empty maps in the site picker) stay
- `ingest` and `upsert_*` reducers accept any caller; restrict them to the ingest service identity (needs a persisted token for `spacetime-ingest`)
- An AGV that never sends a `state` with a position never shows up (no map to place it on); its `connection` messages are dropped
- MQTT source never unsubscribes: a site removed from config stays in the persistent session, so this instance keeps taking its share of `$share/{group}/...` messages, rejects them as `unknown_site` and acks them (lost if other instances still handle that site)
- Ingest buffers are in memory: messages buffered when a service process dies are lost (accepted in ADR-0001)
- `spacetime-ingest` shutdown can be SIGKILLed: compose has no `stop_grace_period` (Docker default 10 s), but MQTT stop + `shutdownFlushSecs` (10 s) + metrics flush (up to 5 s) is longer, so the last writes and metrics are lost
- `spacetime-ingest` `Config::load` validation has no tests (bad `spacetime.uri`, empty `database`, unknown fields)
- e2e `keeps_data_through_a_tdengine_outage` failed once (an `eventually` timed out, panic message lost by the output filter) and passed on the next two runs; find the flaky step. It failed once more in a full run and passed alone
- e2e `stores_only_the_configured_sites` and `replicas_share_the_work_without_duplicates` time out at their SpacetimeDB step ("every AGV ... online"), before any TDengine check; not checked against a clean checkout
- `tdengine-ingest`: one batch is still one statement per connection; bind parameters (stmt2) would skip SQL parsing entirely

## Ideas

- Broker URL per site in the ingest config, for one broker per site
