// Publishes a fresh `asimov-e2e` database and fills it with a known fleet:
// berlin-warehouse has 12 AGVs (two pages of 10), paris-warehouse has 2.
// `acme/AGV-02` has an obstacle error.

import { publish, upsertAgv, upsertConnection, upsertMap, upsertState } from "./spacetime";

export default function globalSetup() {
  publish();

  const add = (mapId: string, manufacturer: string, serialNumber: string, errorType?: string) => {
    upsertAgv(manufacturer, serialNumber, mapId);
    upsertState(manufacturer, serialNumber, errorType);
    upsertConnection(manufacturer, serialNumber, "online");
  };

  upsertMap("berlin-warehouse");
  upsertMap("paris-warehouse");
  for (let i = 1; i <= 12; i++) {
    const serial = `AGV-${String(i).padStart(2, "0")}`;
    add("berlin-warehouse", "acme", serial, i === 2 ? "obstacleDetected" : undefined);
  }
  add("paris-warehouse", "beta", "P-1");
  add("paris-warehouse", "beta", "P-2");
}
