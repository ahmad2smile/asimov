// Reducers that keep the map, AGV and AGV state tables current. Callers
// identify things by their VDA 5050 names.
//
// - `ingest` takes a batch of VDA 5050 messages (from `spacetime-ingest`) and
//   does everything itself: registers maps and AGVs from a `state` with a
//   position, and stores a `connection` by AGV name, registered or not.
// - `upsert_*` write one thing each. Maps and AGVs are registered first
//   (`upsert_map`, `upsert_agv`); a state update fails with "not found" for
//   an AGV or map that is not. A connection update needs only a valid name.
//
// State and connection take the message's VDA 5050 header timestamp
// (`sentAt`) and ignore a message older than the one stored, since several
// ingest instances can deliver one AGV's messages out of order.

import type { Infer, Timestamp } from "spacetimedb";
import {
  SenderError,
  t,
  type InferSchema,
  type ReducerCtx,
} from "spacetimedb/server";
import spacetimedb, { AgvStateFields, ConnectionState } from "./schema";

type Ctx = ReducerCtx<InferSchema<typeof spacetimedb>>;

// `manufacturer/serialNumber`. Undefined when a part is empty or the
// manufacturer holds a `/`, which would let two AGVs share one id.
const agvIdOf = (manufacturer: string, serialNumber: string) =>
  manufacturer && serialNumber && !manufacturer.includes("/")
    ? `${manufacturer}/${serialNumber}`
    : undefined;

// True when a message sent at `sentAt` is older than the stored one.
const isStale = (stored: Timestamp | undefined, sentAt: Timestamp) =>
  stored != null && stored.microsSinceUnixEpoch > sentAt.microsSinceUnixEpoch;

// Registers an AGV on a map, or moves a known AGV to it.
const placeAgv = (
  ctx: Ctx,
  manufacturer: string,
  serialNumber: string,
  mapId: string,
) => {
  const agvId = `${manufacturer}/${serialNumber}`;

  const known = ctx.db.agv.agvId.find(agvId);
  if (known) {
    if (known.mapId !== mapId) ctx.db.agv.agvId.update({ ...known, mapId });
    return;
  }

  ctx.db.agv.insert({ agvId, manufacturer, serialNumber, mapId });
};

const writeState = (
  ctx: Ctx,
  agvId: string,
  state: Infer<typeof AgvStateFields>,
  sentAt: Timestamp,
) => {
  const known = ctx.db.agvState.agvId.find(agvId);
  if (isStale(known?.stateSentAt, sentAt)) {
    console.warn(`dropped stale state of ${agvId}`);
    return;
  }

  const row = {
    ...state,
    agvId,
    updatedAt: ctx.timestamp,
    stateSentAt: sentAt,
    alert: state.error != null,
  };

  if (known) {
    ctx.db.agvState.agvId.update(row);
  } else {
    ctx.db.agvState.insert(row);
  }
};

const writeConnection = (
  ctx: Ctx,
  agvId: string,
  connectionState: Infer<typeof ConnectionState>,
  sentAt: Timestamp,
) => {
  const known = ctx.db.agvConnection.agvId.find(agvId);
  if (isStale(known?.sentAt, sentAt)) {
    console.warn(`dropped stale connection of ${agvId}`);
    return;
  }

  const row = { agvId, connectionState, sentAt };

  if (known) {
    ctx.db.agvConnection.agvId.update(row);
  } else {
    ctx.db.agvConnection.insert(row);
  }
};

// One VDA 5050 message, trimmed to what the dashboard needs.
const IngestMessage = t.object("IngestMessage", {
  manufacturer: t.string(),
  serialNumber: t.string(),
  // VDA 5050 header `timestamp`
  sentAt: t.timestamp(),
  body: t.enum("IngestBody", {
    State: t.object("IngestState", {
      // From `agvPosition`; unset when the AGV reports no position.
      mapId: t.option(t.string()),
      state: AgvStateFields,
    }),
    Connection: ConnectionState,
  }),
});

// Never fails for one message, so one odd message cannot block a batch.
export const ingest = spacetimedb.reducer(
  { name: "ingest" },
  { messages: t.array(IngestMessage) },
  (ctx, { messages }) => {
    for (const { manufacturer, serialNumber, sentAt, body } of messages) {
      const agvId = agvIdOf(manufacturer, serialNumber);
      if (!agvId) {
        console.warn(
          `dropped message with invalid AGV name: '${manufacturer}' '${serialNumber}'`,
        );
        continue;
      }

      if (body.tag === "State") {
        const { mapId, state } = body.value;

        // An older state must not move the AGV back to an older map.
        if (isStale(ctx.db.agvState.agvId.find(agvId)?.stateSentAt, sentAt)) {
          console.warn(`dropped stale state of ${agvId}`);
          continue;
        }

        if (mapId != null) {
          if (!ctx.db.map.mapId.find(mapId)) {
            ctx.db.map.insert({ mapId, name: mapId });
          }

          placeAgv(ctx, manufacturer, serialNumber, mapId);
        }

        if (ctx.db.agv.agvId.find(agvId)) {
          writeState(ctx, agvId, state, sentAt);
        } else {
          // No position and not registered: no map to show it on.
          console.warn(
            `dropped state of ${agvId}: no position and not registered`,
          );
        }
      } else {
        writeConnection(ctx, agvId, body.value, sentAt);
      }
    }
  },
);

export const upsertMap = spacetimedb.reducer(
  { name: "upsert_map" },
  {
    // VDA 5050 `mapId`
    mapId: t.string(),
    // Human readable; defaults to `mapId` for a new map, unchanged if unset.
    name: t.option(t.string()),
  },
  (ctx, { mapId, name }) => {
    const known = ctx.db.map.mapId.find(mapId);
    if (!known) {
      ctx.db.map.insert({ mapId, name: name ?? mapId });
    } else if (name != null) {
      ctx.db.map.mapId.update({ ...known, name });
    }
  },
);

// Registers an AGV on a map, or moves a known AGV to it.
export const upsertAgv = spacetimedb.reducer(
  { name: "upsert_agv" },
  {
    manufacturer: t.string(),
    serialNumber: t.string(),
    mapId: t.string(),
  },
  (ctx, { manufacturer, serialNumber, mapId }) => {
    if (!agvIdOf(manufacturer, serialNumber)) {
      throw new SenderError(
        `invalid AGV name: '${manufacturer}' '${serialNumber}'`,
      );
    }
    if (!ctx.db.map.mapId.find(mapId)) {
      throw new SenderError(`map not found: ${mapId}`);
    }

    placeAgv(ctx, manufacturer, serialNumber, mapId);
  },
);

// Throws when the AGV or its map is not registered.
const registeredAgv = (
  ctx: Ctx,
  manufacturer: string,
  serialNumber: string,
) => {
  const agvId = agvIdOf(manufacturer, serialNumber);
  if (!agvId)
    throw new SenderError(
      `invalid AGV name: '${manufacturer}' '${serialNumber}'`,
    );

  const agv = ctx.db.agv.agvId.find(agvId);
  if (!agv) throw new SenderError(`AGV not found: ${agvId}`);
  if (!ctx.db.map.mapId.find(agv.mapId)) {
    throw new SenderError(`map not found for AGV: ${agvId}`);
  }
  return agvId;
};

export const upsertAgvState = spacetimedb.reducer(
  { name: "upsert_agv_state" },
  {
    manufacturer: t.string(),
    serialNumber: t.string(),
    state: AgvStateFields,
    sentAt: t.timestamp(),
  },
  (ctx, { manufacturer, serialNumber, state, sentAt }) => {
    writeState(
      ctx,
      registeredAgv(ctx, manufacturer, serialNumber),
      state,
      sentAt,
    );
  },
);

export const upsertAgvConnection = spacetimedb.reducer(
  { name: "upsert_agv_connection" },
  {
    manufacturer: t.string(),
    serialNumber: t.string(),
    connectionState: ConnectionState,
    sentAt: t.timestamp(),
  },
  (ctx, { manufacturer, serialNumber, connectionState, sentAt }) => {
    const agvId = agvIdOf(manufacturer, serialNumber);
    if (!agvId) {
      throw new SenderError(
        `invalid AGV name: '${manufacturer}' '${serialNumber}'`,
      );
    }

    writeConnection(ctx, agvId, connectionState, sentAt);
  },
);
