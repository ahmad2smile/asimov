#!/usr/bin/env bash
# Writes one snapshot of the simulated fleet into a SpacetimeDB database
# through the upsert reducers. Safe to rerun: rows are upserted.
# Usage: scripts/seed-spacetime.sh [database]   (default: asimov on local)
set -euo pipefail

DB="${1:-asimov}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

cargo run --quiet --release --manifest-path "$ROOT/simulator/Cargo.toml" --bin seed |
  while IFS=$'\t' read -r reducer args; do
    IFS=$'\t' read -ra args <<<"$args"
    # --no-config: `spacetime.json` would otherwise read "$DB" as the reducer name.
    spacetime call "$DB" --server local --no-config "$reducer" "${args[@]}" >/dev/null
  done

spacetime sql "$DB" --server local --no-config "SELECT * FROM fleet_stats" 2>/dev/null
