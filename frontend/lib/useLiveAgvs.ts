import { useEffect, useMemo, useState } from "react";
import { useSpacetimeDB, useTable } from "spacetimedb/react";
import { tables } from "@/module_bindings";
import { toAgv, type Agv } from "./fleet";

const anyOf = <E extends { or(other: E): E }>(exprs: E[]) =>
  exprs.reduce((a, b) => a.or(b));

/**
 * Subscribes to exactly these AGVs' `agv`, `agv_state` and `agv_connection` rows and
 * returns them live, in `ids` order. While a new id set is being subscribed,
 * the previous result is kept so the UI doesn't flash empty.
 */
export function useLiveAgvs(ids: readonly string[]): {
  agvs: Agv[];
  ready: boolean;
} {
  const enabled = ids.length > 0;
  // Same set in another order is the same subscription.
  const key = JSON.stringify([...ids].sort());

  // The queries are built even while disabled, and an OR chain needs at least
  // one term, so a disabled hook uses a placeholder id that is never read.
  const { agvQuery, stateQuery, connectionQuery } = useMemo(() => {
    const terms: string[] = JSON.parse(key);
    if (terms.length === 0) terms.push("");
    return {
      agvQuery: tables.agv.where((r) =>
        anyOf(terms.map((id) => r.agvId.eq(id))),
      ),
      stateQuery: tables.agvState.where((r) =>
        anyOf(terms.map((id) => r.agvId.eq(id))),
      ),
      connectionQuery: tables.agvConnection.where((r) =>
        anyOf(terms.map((id) => r.agvId.eq(id))),
      ),
    };
  }, [key]);

  const [agvRows] = useTable(agvQuery, { enabled });
  const [states] = useTable(stateQuery, { enabled });
  const [connections] = useTable(connectionQuery, { enabled });
  // One row per site, so the whole table is small.
  const [maps, mapsReady] = useTable(tables.map);

  // `useTable` never resets its `ready` when the query changes, so track which
  // id set has been applied by the server.
  const { isActive, getConnection } = useSpacetimeDB();
  const [appliedKey, setAppliedKey] = useState<string>();

  useEffect(() => {
    const connection = getConnection();

    if (!enabled || !isActive || !connection) return;

    let current = true;
    const subscription = connection
      .subscriptionBuilder()
      .onApplied(() => current && setAppliedKey(key))
      .subscribe([agvQuery, stateQuery, connectionQuery]);

    return () => {
      current = false;
      // This set is no longer applied, even if it is subscribed again later.
      setAppliedKey(undefined);
      subscription.unsubscribe();
    };
  }, [enabled, isActive, getConnection, key, agvQuery, stateQuery, connectionQuery]);

  const ready = mapsReady && (!enabled || appliedKey === key);

  const agvs = useMemo(() => {
    const agvById = new Map(agvRows.map((a) => [a.agvId, a]));
    const stateById = new Map(states.map((s) => [s.agvId, s]));
    const connectionById = new Map(connections.map((c) => [c.agvId, c]));
    const mapById = new Map(maps.map((m) => [m.mapId, m]));

    return ids.flatMap((id) => {
      const agv = agvById.get(id);
      const state = stateById.get(id);
      const map = agv && mapById.get(agv.mapId);

      return agv && state && map ? [toAgv(agv, map, state, connectionById.get(id))] : [];
    });
  }, [ids, agvRows, states, connections, maps]);

  // Last result of a ready set, shown until the next set is ready.
  const [shown, setShown] = useState<Agv[]>([]);

  if (ready && shown !== agvs) {
    setShown(agvs);
  }

  return { agvs: ready ? agvs : shown, ready };
}
