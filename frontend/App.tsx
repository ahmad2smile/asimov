import { useMemo, useState } from "react";
import { useTable } from "spacetimedb/react";
import { tables } from "./module_bindings";
import { AgvTable } from "@/components/fleet/AgvTable";
import { FleetFilters } from "@/components/fleet/FleetFilters";
import { FleetHeader } from "@/components/fleet/FleetHeader";
import { Pager } from "@/components/fleet/Pager";
import { StatCards } from "@/components/fleet/StatCards";
import { PAGE_SIZE, useAgvPaginated } from "@/lib/useAgvPage";
import { useLiveAgvs } from "@/lib/useLiveAgvs";
import Details, { useAgvRoute } from "./Details";

const NO_IDS: string[] = [];

export default function App() {
  const { agvId, open } = useAgvRoute();
  const [fleetStats] = useTable(tables.fleetStats);
  const [maps, mapsReady] = useTable(tables.map);

  const mapIds = useMemo(
    () =>
      [...maps]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((m) => m.mapId),
    [maps],
  );

  const [query, setQuery] = useState("");
  const [pickedMapId, setPickedMapId] = useState<string>();

  // Always exactly one map: the picked one, or the first while none is
  // picked or the picked one is gone.
  const mapId = mapIds.find((id) => id === pickedMapId) ?? mapIds[0];

  // The list stays mounted while an AGV is open, so its filters and page are
  // kept for the way back. It just stops fetching and subscribing.
  const listing = agvId === undefined;
  const stats = fleetStats[0];

  const page = useAgvPaginated({
    mapId,
    search: query.trim(),
    // Refetch when AGVs join or leave the fleet.
    refresh: stats?.agvs,
    enabled: listing,
  });
  // Live rows for exactly the AGVs on this page.
  const { agvs } = useLiveAgvs(listing ? page.agvIds : NO_IDS);

  if (!listing) return <Details key={agvId} agvId={agvId} />;

  return (
    <div className="mx-auto max-w-7xl space-y-6 p-6">
      <FleetHeader />
      <StatCards stats={stats} />
      <FleetFilters
        query={query}
        onQueryChange={setQuery}
        mapId={mapId}
        mapIds={mapIds}
        onMapChange={setPickedMapId}
      />
      {mapsReady && mapIds.length === 0 ? (
        <p className="rounded-xl border bg-card p-8 text-center text-muted-foreground">
          No maps in SpacetimeDB yet.
        </p>
      ) : (
        <AgvTable
          agvs={agvs}
          selectedId={undefined}
          onSelect={open}
          footer={
            <Pager
              pageNumber={page.pageNumber}
              pageSize={PAGE_SIZE}
              count={page.agvIds.length}
              // Only fleet-wide counts exist; a per-site total would need its own count.
              total={undefined}
              hasMore={page.hasMore}
              canGoBack={page.canGoBack}
              onPrevious={page.previous}
              onNext={page.next}
            />
          }
        />
      )}
    </div>
  );
}
