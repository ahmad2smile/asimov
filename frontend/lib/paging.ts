export type Cursor = string | undefined;

/** Cursor stack of the visited pages, valid only for the filter `key` it was built under. */
export interface CursorState {
  key: string;
  stack: Cursor[];
}

/** The first page has no cursor. */
export const FIRST_PAGE: Cursor[] = [undefined];

/** A stack from another filter is ignored, so new filters start from the first page. */
export const cursorsFor = (state: CursorState, key: string): Cursor[] =>
  state.key === key ? state.stack : FIRST_PAGE;

/**
 * Go back only when the page fetched for the current cursor is empty. A page
 * fetched for another cursor is stale and must not pop again.
 */
export const shouldPop = (
  page: { cursor: Cursor; agvIds: readonly string[] } | undefined,
  cursor: Cursor,
  depth: number,
) => page !== undefined && page.cursor === cursor && page.agvIds.length === 0 && depth > 1;
