// Joins an agv row with its agv_state and agv_connection rows and derives what the
// dashboard shows (site, activity, alerts).
import type { Agv as AgvRow, AgvConnection, AgvState, Map as MapRow } from '@/module_bindings/types';
import { activityOf, type Activity } from '../../backend/src/display';

export { type Activity };

export type ConnectionTag = AgvConnection['connectionState']['tag'];

export interface Agv {
  id: string;
  manufacturer: string;
  serialNumber: string;
  /** `map.name`, as stored. */
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
  connectionRow: AgvConnection | undefined,
): Agv {
  const connection = connectionRow?.connectionState.tag ?? 'Offline';
  return {
    id: agv.agvId,
    manufacturer: agv.manufacturer,
    serialNumber: agv.serialNumber,
    siteName: map.name,
    state,
    connection,
    activity: activityOf(connection, state),
  };
}

export const isOnline = (agv: Agv) => agv.connection === 'Online';

/** "obstacleDetected" -> "Obstacle detected" */
export function humanize(camelCase: string): string {
  const words = camelCase.replace(/([a-z])([A-Z])/g, '$1 $2').toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}
