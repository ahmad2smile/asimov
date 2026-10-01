// What the dashboard shows for an AGV's activity. Shared by the frontend (to
// render) and `agv_page` (to search the same text), so search matches exactly
// what is on screen. No imports, so both sides can load it.
//
// A site is shown as its `map.name`, as stored.

export type Activity = 'Offline' | 'Error' | 'Idle';

export function activityOf(state: {
  connectionState: { tag: string };
  error?: { errorLevel: { tag: string } };
}): Activity {
  if (state.connectionState.tag !== 'Online') return 'Offline';
  if (state.error?.errorLevel.tag === 'Fatal') return 'Error';
  return 'Idle';
}
