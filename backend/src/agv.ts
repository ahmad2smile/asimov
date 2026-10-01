// Reducers that keep the map, AGV and AGV state tables current. Callers
// identify things by their VDA 5050 names.
//
// Maps and AGVs are registered first (`upsert_map`, `upsert_agv`). State and
// connection updates fail with "not found" for an AGV or map that is not.

import { SenderError, t } from 'spacetimedb/server';
import spacetimedb, { AgvStateFields, ConnectionState } from './schema';

// State of an AGV whose connection message arrived before its first state.
const BLANK_STATE = {
  orderId: '',
  lastNodeId: '',
  error: undefined,
};
const OFFLINE = { tag: 'Offline' } as const;

export const upsertMap = spacetimedb.reducer(
  { name: 'upsert_map' },
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
  }
);

// Registers an AGV on a map, or moves a known AGV to it.
export const upsertAgv = spacetimedb.reducer(
  { name: 'upsert_agv' },
  {
    manufacturer: t.string(),
    serialNumber: t.string(),
    mapId: t.string(),
  },
  (ctx, { manufacturer, serialNumber, mapId }) => {
    if (!ctx.db.map.mapId.find(mapId)) {
      throw new SenderError(`map not found: ${mapId}`);
    }

    const agvId = `${manufacturer}/${serialNumber}`;
    const known = ctx.db.agv.agvId.find(agvId);
    if (known) {
      ctx.db.agv.agvId.update({ ...known, mapId });
    } else {
      ctx.db.agv.insert({ agvId, manufacturer, serialNumber, mapId });
    }
  }
);

export const upsertAgvState = spacetimedb.reducer(
  { name: 'upsert_agv_state' },
  {
    manufacturer: t.string(),
    serialNumber: t.string(),
    state: AgvStateFields,
  },
  (ctx, { manufacturer, serialNumber, state }) => {
    const agvId = `${manufacturer}/${serialNumber}`;
    const agv = ctx.db.agv.agvId.find(agvId);
    if (!agv) throw new SenderError(`AGV not found: ${agvId}`);
    if (!ctx.db.map.mapId.find(agv.mapId)) {
      throw new SenderError(`map not found for AGV: ${agvId}`);
    }

    const known = ctx.db.agvState.agvId.find(agvId);
    const row = {
      ...state,
      agvId,
      connectionState: known?.connectionState ?? OFFLINE,
      updatedAt: ctx.timestamp,
      alert: state.error != null,
    };
    if (known) {
      ctx.db.agvState.agvId.update(row);
    } else {
      ctx.db.agvState.insert(row);
    }
  }
);

export const upsertAgvConnection = spacetimedb.reducer(
  { name: 'upsert_agv_connection' },
  {
    manufacturer: t.string(),
    serialNumber: t.string(),
    connectionState: ConnectionState,
  },
  (ctx, { manufacturer, serialNumber, connectionState }) => {
    const agvId = `${manufacturer}/${serialNumber}`;
    const agv = ctx.db.agv.agvId.find(agvId);
    if (!agv) throw new SenderError(`AGV not found: ${agvId}`);
    if (!ctx.db.map.mapId.find(agv.mapId)) {
      throw new SenderError(`map not found for AGV: ${agvId}`);
    }

    const known = ctx.db.agvState.agvId.find(agvId);
    if (known) {
      ctx.db.agvState.agvId.update({
        ...known,
        connectionState,
        updatedAt: ctx.timestamp,
      });
    } else {
      ctx.db.agvState.insert({
        ...BLANK_STATE,
        agvId,
        connectionState,
        updatedAt: ctx.timestamp,
        alert: false,
      });
    }
  }
);
