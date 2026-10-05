// Helpers to drive the local SpacetimeDB through the `spacetime` CLI.
// `--no-config` keeps `spacetime.json` from redirecting commands to `asimov`.

import { execFileSync } from "node:child_process";

export const DB = "asimov-e2e";
const BASE = ["--server", "local", "--no-config"];

const spacetime = (...args: string[]) =>
  execFileSync("spacetime", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

export const publish = () =>
  spacetime("publish", DB, ...BASE, "--module-path", "backend", "--delete-data=always", "--yes");

export const remove = () => spacetime("delete", DB, ...BASE, "--yes");

const call = (reducer: string, ...args: unknown[]) =>
  spacetime("call", DB, ...BASE, reducer, ...args.map((a) => JSON.stringify(a)));

// VDA 5050 header timestamp of a message sent now, as SpacetimeDB JSON.
const sentNow = () => ({ __timestamp_micros_since_unix_epoch__: Date.now() * 1000 });

export const upsertMap = (mapId: string, name?: string) =>
  call("upsert_map", mapId, name === undefined ? { none: [] } : { some: name });

export const upsertAgv = (manufacturer: string, serialNumber: string, mapId: string) =>
  call("upsert_agv", manufacturer, serialNumber, mapId);

export const upsertState = (
  manufacturer: string,
  serialNumber: string,
  errorType?: string,
  orderId = "",
) =>
  call("upsert_agv_state", manufacturer, serialNumber, {
    order_id: orderId,
    last_node_id: "",
    driving: false,
    paused: false,
    charging: false,
    error: errorType
      ? { some: { error_type: errorType, error_level: { warning: [] } } }
      : { none: [] },
  }, sentNow());

export const upsertConnection = (manufacturer: string, serialNumber: string, state: string) =>
  call("upsert_agv_connection", manufacturer, serialNumber, { [state]: [] }, sentNow());
