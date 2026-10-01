// VDA 5050 AGV tables. Types mirror the message shapes in `simulator/src/agv.rs`
// (VDA 5050 2.1.0), trimmed to the fields a dashboard needs.

import { schema, table, t } from "spacetimedb/server";

export const ConnectionState = t.enum("ConnectionState", {
  Online: t.unit(),
  Offline: t.unit(),
  Connectionbroken: t.unit(),
});

export const ErrorLevel = t.enum("ErrorLevel", {
  Warning: t.unit(),
  Fatal: t.unit(),
});

const AgvError = t.object("AgvError", {
  // e.g. "obstacleDetected", "batteryLow"
  errorType: t.string(),
  errorLevel: ErrorLevel,
});

// A site's map. `mapId` is the VDA 5050 `mapId`, a free-form unique string;
// everything else refers to the map by it.
export const map = table(
  { name: "map", public: true },
  {
    // e.g. "berlin-warehouse"
    mapId: t.string().primaryKey(),
    // Human readable; defaults to `mapId`.
    name: t.string(),
  },
);

// One row per AGV, created by `upsert_agv`. VDA 5050 identifies an AGV by
// manufacturer + serial number, kept here as the primary key `agvId`.
//
// `by_site` orders a map's AGVs by `agvId` for paging
// (the primary key only supports point lookups).
export const agv = table(
  {
    name: "agv",
    public: true,
    indexes: [
      { accessor: "by_site", algorithm: "btree", columns: ["mapId", "agvId"] },
    ],
  },
  {
    // "<manufacturer>/<serialNumber>"
    agvId: t.string().primaryKey(),
    manufacturer: t.string(),
    serialNumber: t.string(),
    // `map.mapId`
    mapId: t.string(),
  },
);

// The VDA 5050 `state` fields a dashboard needs, as sent by a client.
const stateFields = {
  orderId: t.string(),
  lastNodeId: t.string(),
  // The latest error; unset when the AGV reports none.
  error: t.option(AgvError),
};
export const AgvStateFields = t.object("AgvStateFields", stateFields);

// Latest VDA 5050 `state` and `connection` per AGV, in one row. The row is
// created by whichever message arrives first, so until the other arrives its
// fields hold defaults (`Offline` connection, blank state). The high-rate
// `visualization` history goes to TDengine, not here.
export const agvState = table(
  { name: "agv_state", public: true },
  {
    // `agv.agvId`
    agvId: t.string().primaryKey(),
    ...stateFields,
    // Indexed so `fleet_stats` can count online AGVs.
    connectionState: ConnectionState.index("btree"),
    // Last write of either message. Set by the reducer.
    updatedAt: t.timestamp(),
    // `error` is set; indexed so `fleet_stats` can count alerts.
    alert: t.bool().index("btree"),
  },
);

const spacetimedb = schema({ map, agv, agvState });
export default spacetimedb;
