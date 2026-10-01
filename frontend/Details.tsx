import { useEffect, useMemo, useState } from "react";
import { ArrowLeft } from "lucide-react";
import { AgvDetail } from "@/components/fleet/AgvDetail";
import { useLiveAgvs } from "@/lib/useLiveAgvs";

const AGV_ROUTE = "#/agv/";

/** Selected AGV id from the URL hash (`#/agv/<id>`), so back/forward and deep links work. */
export function useAgvRoute() {
  const read = () =>
    location.hash.startsWith(AGV_ROUTE)
      ? decodeURIComponent(location.hash.slice(AGV_ROUTE.length))
      : undefined;
  const [agvId, setAgvId] = useState(read);

  useEffect(() => {
    const onChange = () => setAgvId(read());
    window.addEventListener("hashchange", onChange);
    // The hash may have changed between render and this effect.
    onChange();
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  const open = (id: string) =>
    (location.hash = AGV_ROUTE + encodeURIComponent(id));
  return { agvId, open };
}

export default function Details({ agvId }: { agvId: string }) {
  // Only this AGV is subscribed.
  const ids = useMemo(() => [agvId], [agvId]);
  const { agvs, ready } = useLiveAgvs(ids);
  const agv = agvs[0];
  return (
    <div className="mx-auto max-w-7xl space-y-6 p-6">
      <a
        href="#/"
        className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="size-4" /> Back to fleet
      </a>
      {agv ? (
        <div className="max-w-sm">
          <AgvDetail agv={agv} />
        </div>
      ) : (
        <p className="rounded-xl border bg-card p-8 text-center text-muted-foreground">
          {ready ? `AGV ${agvId} not found.` : "Loading…"}
        </p>
      )}
    </div>
  );
}
