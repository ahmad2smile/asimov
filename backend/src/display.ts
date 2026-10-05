// What the dashboard shows for an AGV's activity. Shared by the frontend (to
// render) and `agv_page` (to search the same text), so search matches exactly
// what is on screen. No imports, so both sides can load it.
//
// A site is shown as its `map.name`, as stored.

export type Activity = 'Offline' | 'Error' | 'Charging' | 'Driving' | 'Paused' | 'Idle';

// `connection` is the stored connection tag; unset until the AGV has sent one.
// The first match wins, so a fatal error shows even while the AGV drives.
export function activityOf(
  connection: string | undefined,
  state:
    | {
        driving: boolean;
        paused: boolean;
        charging: boolean;
        error?: { errorLevel: { tag: string } };
      }
    | undefined,
): Activity {
  if (connection !== 'Online') return 'Offline';
  if (state?.error?.errorLevel.tag === 'Fatal') return 'Error';
  if (state?.charging) return 'Charging';
  if (state?.paused) return 'Paused';
  if (state?.driving) return 'Driving';
  return 'Idle';
}
