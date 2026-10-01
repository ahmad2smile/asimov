import { useEffect, useMemo, useState } from "react";
import { useProcedure } from "spacetimedb/react";
import { procedures } from "@/module_bindings";
import type { AgvPage } from "@/module_bindings/types";
import { cursorsFor, FIRST_PAGE, shouldPop, type Cursor, type CursorState } from "./paging";

export const PAGE_SIZE = 10;

/**
 * Server-side cursor paging of AGV ids for one site and search. A new filter
 * starts from the first page, and a change of `refresh` refetches the current
 * page. While disabled nothing is fetched, but the page position is kept.
 */
export function useAgvPaginated({
  mapId,
  search,
  refresh,
  enabled,
}: {
  mapId: string | undefined;
  search: string;
  refresh: unknown;
  enabled: boolean;
}) {
  // Cursor per visited page, valid only for the filter it was built under.
  const filterKey = JSON.stringify([search, mapId]);
  const [cursorState, setCursorState] = useState<CursorState>({
    key: filterKey,
    stack: FIRST_PAGE,
  });
  const cursors = cursorsFor(cursorState, filterKey);
  const cursor = cursors[cursors.length - 1];
  const setCursors = (change: (current: Cursor[]) => Cursor[]) =>
    setCursorState((state) => ({
      key: filterKey,
      stack: change(cursorsFor(state, filterKey)),
    }));
  const previous = () => setCursors((current) => current.slice(0, -1));

  const agvPage = useProcedure(procedures.agvPage);
  // The cursor tells which request the page answers.
  const [page, setPage] = useState<{ cursor: Cursor } & AgvPage>();
  useEffect(() => {
    if (!enabled || mapId === undefined) return;
    let current = true;
    agvPage({ mapId, search, afterAgvId: cursor, limit: PAGE_SIZE }).then(
      (result) => current && setPage({ ...result, cursor }),
      console.error,
    );
    return () => {
      current = false;
    };
  }, [agvPage, enabled, mapId, search, cursor, refresh]);

  // If rows disappear and leave this cursor page empty, return to the prior page.
  useEffect(() => {
    if (shouldPop(page, cursor, cursors.length)) previous();
  }, [page, cursor, cursors.length]);

  const next = () => {
    const nextAgvId = page?.nextAgvId;
    if (nextAgvId !== undefined) setCursors((current) => [...current, nextAgvId]);
  };

  // Stable while the page is unchanged, so live subscriptions are not rebuilt.
  const agvIds = useMemo(() => page?.agvIds ?? [], [page]);

  return {
    agvIds,
    pageNumber: cursors.length,
    hasMore: page?.nextAgvId !== undefined,
    canGoBack: cursors.length > 1,
    next,
    previous,
  };
}
