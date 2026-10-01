// Publishes a fresh `asimov-e2e` database and fills it with a known fleet:
// berlin-warehouse ("Berlin Central Hub") has 12 AGVs (two pages of 10),
// paris-warehouse (no name, so shown as its mapId) has 2.
// `acme/AGV-02` has an obstacle error, `acme/AGV-05` runs order `order-77`,
// and `acme/AGV-07` is offline.

import { publish, upsertAgv, upsertConnection, upsertMap, upsertState } from "./spacetime";

export default function globalSetup() {
  publish();

  const add = (
    mapId: string,
    manufacturer: string,
    serialNumber: string,
    { errorType, orderId, connection = "online" }: {
      errorType?: string;
      orderId?: string;
      connection?: string;
    } = {},
  ) => {
    upsertAgv(manufacturer, serialNumber, mapId);
    upsertState(manufacturer, serialNumber, errorType, orderId);
    upsertConnection(manufacturer, serialNumber, connection);
  };

  upsertMap("berlin-warehouse", "Berlin Central Hub");
  upsertMap("paris-warehouse");
  for (let i = 1; i <= 12; i++) {
    const serial = `AGV-${String(i).padStart(2, "0")}`;
    add("berlin-warehouse", "acme", serial, {
      errorType: i === 2 ? "obstacleDetected" : undefined,
      orderId: i === 5 ? "order-77" : undefined,
      connection: i === 7 ? "offline" : "online",
    });
  }
  add("paris-warehouse", "beta", "P-1");
  add("paris-warehouse", "beta", "P-2");
}
