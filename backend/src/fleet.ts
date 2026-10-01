// Dashboard read side, built so no call scans more than one page of AGVs.
//
// - `agv_page` finds which AGVs are on a page; clients then subscribe to
//   exactly those `agv_state` rows for live updates.
// - `fleet_stats` is a live fleet-wide count of maps, AGVs, online AGVs, and
//   alerts, read from table counts and indexes.

import { Range, t, type Bound } from "spacetimedb/server";
import spacetimedb from "./schema";

const MAX_PAGE_SIZE = 100;

const AgvPage = t.object("AgvPage", {
  // In `agvId` order.
  agvIds: t.array(t.string()),
  // Cursor to pass back as `afterAgvId` for the next page; absent on the
  // last page.
  nextAgvId: t.option(t.string()),
});

// AGVs on one page of one map, ordered by `agvId` ("<manufacturer>/<serialNumber>").
// `search` is a case-sensitive prefix of the `agvId`; `afterAgvId` is the cursor
// from the previous page. One `by_site` index seek plus one page, independent
// of page depth.
export const agvPage = spacetimedb.procedure(
  { name: "agv_page" },
  {
    mapId: t.string(),
    search: t.string(),
    afterAgvId: t.option(t.string()),
    limit: t.u32(),
  },
  AgvPage,
  (ctx, { mapId, search, afterAgvId, limit }) => {
    const prefix = search.trim();
    const size = Math.min(Math.max(limit, 1), MAX_PAGE_SIZE);
    // A cursor before the search prefix (stale, or from another search) is ignored.
    const from: Bound<string> =
      afterAgvId != null && afterAgvId >= prefix
        ? { tag: "excluded", value: afterAgvId }
        : { tag: "included", value: prefix };

    return ctx.withTx((tx) => {
      const agvIds: string[] = [];

      for (const a of tx.db.agv.by_site.filter([mapId, new Range(from)])) {
        if (!a.agvId.startsWith(prefix) || agvIds.length > size) break;
        agvIds.push(a.agvId);
      }

      const nextAgvId = agvIds.length > size ? agvIds[size - 1] : undefined;
      return { agvIds: agvIds.slice(0, size), nextAgvId };
    });
  },
);

const FleetStatsRow = t.row("FleetStatsRow", {
  sites: t.u64(),
  agvs: t.u64(),
  online: t.u64(),
  alerts: t.u64(),
});

const count = (rows: Iterable<unknown>) => {
  let n = 0n;
  for (const _ of rows) n++;
  return n;
};

// Fleet-wide counts, rerun after every write to the tables it reads.
// `sites` and `agvs` are O(1); `online` and `alerts` walk their matching
// index entries.
export const fleetStats = spacetimedb.anonymousView(
  { name: "fleet_stats", public: true },
  t.option(FleetStatsRow),
  (ctx) => ({
    sites: ctx.db.map.count(),
    agvs: ctx.db.agv.count(),
    // NOTE: Not great but its in Memory scan of Index on highly optimized spacetimedb collection
    online: count(ctx.db.agvState.connectionState.filter({ tag: "Online" })),
    alerts: count(ctx.db.agvState.alert.filter(true)),
  }),
);
