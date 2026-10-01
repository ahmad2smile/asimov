// Joins an agv row with its agv_state row and derives what the
// dashboard shows (site, activity, alerts).
import type { Agv as AgvRow, AgvState, Map as MapRow } from '@/module_bindings/types';

export type ConnectionTag = AgvState['connectionState']['tag'];

export type Activity =
  | 'Offline'
  | 'Error'
  | 'Idle';

export interface Agv {
  id: string;
  manufacturer: string;
  serialNumber: string;
  siteId: string;
  siteName: string;
  state: AgvState;
  /** `Offline` until the AGV has published a connection message. */
  connection: ConnectionTag;
  activity: Activity;
}

export function toAgv(
  agv: AgvRow,
  map: MapRow,
  state: AgvState,
): Agv {
  const siteId = siteIdOf(map.mapId);
  const connection = state.connectionState.tag;
  return {
    id: agv.agvId,
    manufacturer: agv.manufacturer,
    serialNumber: agv.serialNumber,
    siteId,
    siteName: siteNameOf(siteId),
    state,
    connection,
    activity: activityOf(state, connection),
  };
}

/** The simulator names maps "<siteId>-warehouse". */
export function siteIdOf(mapId: string): string {
  return mapId.replace(/-warehouse$/, '');
}

export function siteNameOf(siteId: string): string {
  return siteId.charAt(0).toUpperCase() + siteId.slice(1);
}

function activityOf(state: AgvState, connection: ConnectionTag): Activity {
  if (connection !== 'Online') return 'Offline';
  if (state.error?.errorLevel.tag === 'Fatal') return 'Error';
  return 'Idle';
}

export const isOnline = (agv: Agv) => agv.connection === 'Online';

/** "obstacleDetected" -> "Obstacle detected" */
export function humanize(camelCase: string): string {
  const words = camelCase.replace(/([a-z])([A-Z])/g, '$1 $2').toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}
