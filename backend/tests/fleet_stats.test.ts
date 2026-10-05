// End-to-end test of the `fleet_stats` view and the `agv_page` procedure:
// publishes the module to a throwaway database on the local server, drives the
// upsert reducers, and checks the results after each step. Needs
// `npm run infra:up`. Run with `npm run spacetime:test`.
// `--no-config` keeps `spacetime.json` from redirecting commands to `asimov`.

import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const DB = 'asimov-test-fleet-stats';
const MODULE = fileURLToPath(new URL('..', import.meta.url));
const BASE = ['--server', 'local', '--no-config'];

const spacetime = (...args: string[]) =>
  execFileSync('spacetime', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });

const call = (reducer: string, ...args: unknown[]) =>
  spacetime('call', DB, ...BASE, reducer, ...args.map(a => JSON.stringify(a)));

// agvName = "<manufacturer>/<serialNumber>"
const split = (agvName: string) => {
  const i = agvName.indexOf('/');
  return [agvName.slice(0, i), agvName.slice(i + 1)];
};

// Registers the map and the AGV on it, as the first message of an AGV does.
const register = (agvName: string, mapId: string) => {
  call('upsert_map', mapId, { none: [] });
  call('upsert_agv', ...split(agvName), mapId);
};

// Name of the map with this `mapId`, as the SQL output prints it.
const mapName = (mapId: string) =>
  spacetime('sql', DB, ...BASE, `SELECT name FROM map WHERE map_id = '${mapId}'`)
    .split('\n')[2]
    ?.trim();

// VDA 5050 header timestamp as SpacetimeDB JSON. Each call gets a later one
// unless `at` is given.
let clock = 1_700_000_000_000_000;
const sentAt = (at = (clock += 1_000_000)) => ({ __timestamp_micros_since_unix_epoch__: at });

// `error` is the latest error, or none.
const state = (agvName: string, error: unknown = NO_ERROR, at?: number, orderId = '', flags = {}) =>
  call('upsert_agv_state', ...split(agvName), {
    order_id: orderId,
    last_node_id: '',
    driving: false,
    paused: false,
    charging: false,
    ...flags,
    error,
  }, sentAt(at));

const connection = (agvName: string, connectionState: string, at?: number) =>
  call('upsert_agv_connection', ...split(agvName), { [connectionState]: [] }, sentAt(at));

// One column of an AGV's `agv_state` row, as the SQL output prints it.
const stateColumn = (agvName: string, column: string) =>
  spacetime('sql', DB, ...BASE, `SELECT ${column} FROM agv_state WHERE agv_id = '${agvName}'`)
    .split('\n')[2]
    ?.trim();

// Connection state of an AGV's `agv_connection` row, as the SQL output prints it.
const connectionOf = (agvName: string) =>
  spacetime('sql', DB, ...BASE, `SELECT connection_state FROM agv_connection WHERE agv_id = '${agvName}'`)
    .split('\n')[2]
    ?.trim();

// Runs `fn` and reports whether it failed with an error matching `pattern`.
const expectNotFound = (label: string, fn: () => unknown, pattern = /not found/) => {
  try {
    fn();
    report(false, label, 'expected an error, the call succeeded');
  } catch (e) {
    const out = String((e as { stderr?: unknown }).stderr ?? e);
    report(pattern.test(out), label, `expected ${pattern}, got '${out}'`);
  }
};

const NO_ERROR = { none: [] };
const ERROR = {
  some: { error_type: 'obstacleDetected', error_level: { warning: [] } },
};
let failures = 0;

const report = (ok: boolean, label: string, detail: string) => {
  if (ok) {
    console.log(`ok   ${label}`);
  } else {
    console.log(`FAIL ${label}: ${detail}`);
    failures++;
  }
};

// `want` is "<sites> <agvs> <online> <alerts>"
const expectStats = (label: string, want: string) => {
  const got = spacetime('sql', DB, ...BASE, 'SELECT * FROM fleet_stats')
    .split('\n')
    .slice(2)
    .map(line => line.split('|'))
    .filter(cols => cols.length === 4)
    .map(cols => cols.map(c => c.trim()).join(' '))
    .join('\n');
  report(got === want, label, `expected '${want}' (sites agvs online alerts), got '${got}'`);
};

const expectEqual = (label: string, got: unknown, want: unknown) =>
  report(got === want, label, `expected '${want}', got '${got}'`);

const check = (label: string, actual: string, wantSubstring: string) =>
  report(actual.includes(wantSubstring), label, `expected '${wantSubstring}' in '${actual}'`);

spacetime('publish', DB, ...BASE, '--module-path', MODULE, '--delete-data=always', '--yes');

try {
  expectStats('empty database', '0 0 0 0');

  expectNotFound('state needs a registered AGV', () => state('a/1'));
  expectNotFound(
    'connection needs a valid AGV name',
    () => call('upsert_agv_connection', 'a/b', '1', { online: [] }, sentAt()),
    /invalid AGV name/,
  );
  expectNotFound('AGV needs a registered map', () => call('upsert_agv', 'a', '1', 'm1'));
  expectStats('failed calls change nothing', '0 0 0 0');

  // A connection carries only the AGV name, so it is stored before the AGV is registered.
  connection('a/1', 'online');
  expectEqual('connection of an unregistered AGV is stored', connectionOf('a/1'), '(online = ())');
  expectStats('an unregistered AGV is not counted', '0 0 0 0');

  register('a/1', 'm1');
  state('a/1');
  expectStats('one online AGV, connection stored before registration', '1 1 1 0');

  register('b/2', 'm2');
  connection('b/2', 'online');
  expectStats('second AGV online', '2 2 2 0');

  state('b/2', ERROR);
  expectStats('state with an error', '2 2 2 1');

  state('a/1');
  expectStats('update does not add an AGV or map', '2 2 2 1');

  state('b/2');
  connection('a/1', 'connectionbroken');
  expectStats('alert cleared, AGV offline', '2 2 1 0');

  call('upsert_agv', 'b', '2', 'm1');
  state('b/2', ERROR);
  expectStats('moving an AGV keeps the counts', '2 2 1 1');

  register('c/3', 'm2');
  state('c/3');
  expectStats('state before connection counts as offline', '2 3 1 1');

  expectEqual('map name defaults to its id', mapName('m1'), '"m1"');
  call('upsert_map', 'm1', { some: 'Main Hall' });
  expectEqual('upsert_map sets a name', mapName('m1'), '"Main Hall"');
  call('upsert_map', 'm1', { none: [] });
  expectEqual('upsert_map without a name keeps it', mapName('m1'), '"Main Hall"');

  const page = (search: string, afterName: unknown, limit: number) =>
    spacetime(
      'call', DB, ...BASE, 'agv_page',
      JSON.stringify('m1'), JSON.stringify(search), JSON.stringify(afterName), String(limit),
    );
  check('page 1 has a cursor', page('', { none: [] }, 1), '[0,"a/1"]');
  check('last page has no cursor', page('', { some: 'a/1' }, 1), '[1,[]]');
  // m1 now holds a/1 (connection broken, so "Offline") and b/2 (online with
  // a warning, so "Idle", on order "ord-9"). Its site name is "Main Hall".
  state('b/2', ERROR, undefined, 'ord-9');
  check('search matches the AGV id', page('b/', { none: [] }, 5), '[["b/2"],');
  check('search matches part of the AGV id', page('/1', { none: [] }, 5), '[["a/1"],');
  check('search matches the order', page('ord-9', { none: [] }, 5), '[["b/2"],');
  check('search matches the activity', page('Offline', { none: [] }, 5), '[["a/1"],');
  check('search matches the site', page('Main Hall', { none: [] }, 5), '[["a/1","b/2"],');
  state('a/1', NO_ERROR, undefined, '', { driving: true });
  connection('a/1', 'online');
  check('search matches Driving', page('Driving', { none: [] }, 5), '[["a/1"],');
  state('a/1', NO_ERROR, undefined, '', { driving: true, paused: true });
  check('paused wins over driving', page('Paused', { none: [] }, 5), '[["a/1"],');
  state('a/1', NO_ERROR, undefined, '', { driving: true, paused: true, charging: true });
  check('charging wins over paused', page('Charging', { none: [] }, 5), '[["a/1"],');
  state('a/1', { some: { error_type: 'x', error_level: { fatal: [] } } }, undefined, '', { charging: true });
  check('a fatal error wins over charging', page('Error', { none: [] }, 5), '[["a/1"],');
  state('a/1');
  connection('a/1', 'connectionbroken');
  check('search is case-sensitive', page('offline', { none: [] }, 5), '[[],');
  check('page after the cursor', page('', { some: 'a/1' }, 5), '[["b/2"],');
  check('page after the last match is empty', page('a/', { some: 'b/2' }, 5), '[[],');

  // Messages of one AGV can arrive out of order (several ingest instances).
  register('d/4', 'm2');
  const t0 = clock;
  state('d/4', NO_ERROR, t0 + 10_000_000, 'new-order');
  state('d/4', NO_ERROR, t0 + 5_000_000, 'old-order');
  expectEqual('older state is ignored', stateColumn('d/4', 'order_id'), '"new-order"');
  state('d/4', NO_ERROR, t0 + 10_000_000, 'same-time');
  expectEqual('state with the same time is written', stateColumn('d/4', 'order_id'), '"same-time"');
  connection('d/4', 'online', t0 + 10_000_000);
  connection('d/4', 'connectionbroken', t0 + 5_000_000);
  expectStats('older connection is ignored', '2 4 2 1');
  connection('d/4', 'offline', t0 + 2_000_000);
  state('d/4', NO_ERROR, t0 + 20_000_000, 'newer-order');
  expectEqual('state and connection times are separate', stateColumn('d/4', 'order_id'), '"newer-order"');
  expectStats('connection still online', '2 4 2 1');

  // `ingest`: batches as `spacetime-ingest` sends them.
  const [manufacturer, serial_number] = split('e/5');
  const ingestState = (mapId: string | null, at: number, orderId: string) => ({
    manufacturer, serial_number, sent_at: sentAt(at),
    body: { state: {
      map_id: mapId == null ? { none: [] } : { some: mapId },
      state: { order_id: orderId, last_node_id: '', driving: false, paused: false, charging: false, error: NO_ERROR },
    } },
  });
  const ingestConnection = (connectionState: string, at: number) => ({
    manufacturer, serial_number, sent_at: sentAt(at), body: { connection: { [connectionState]: [] } },
  });
  const ingest = (...messages: unknown[]) => call('ingest', messages);
  const agvMap = () =>
    spacetime('sql', DB, ...BASE, "SELECT map_id FROM agv WHERE agv_id = 'e/5'").split('\n')[2]?.trim();

  const t1 = clock + 100_000_000;
  ingest(ingestConnection('online', t1), ingestState(null, t1 + 1, 'no-position'));
  expectStats('a state without a position is dropped, an unregistered AGV is not counted', '2 4 2 1');
  expectEqual('its connection is stored', connectionOf('e/5'), '(online = ())');
  ingest(ingestState('m3', t1 + 2, 'first'));
  expectStats('a state with a position registers the map and the AGV, already online', '3 5 3 1');
  expectEqual('AGV on its map', agvMap(), '"m3"');
  ingest(ingestState('m1', t1 + 1, 'older'));
  expectEqual('an older state does not move the AGV', agvMap(), '"m3"');
  expectEqual('an older state is ignored', stateColumn('e/5', 'order_id'), '"first"');
  ingest(ingestState('m1', t1 + 3, 'moved'), ingestConnection('offline', t1 + 3));
  expectEqual('a newer state moves the AGV', agvMap(), '"m1"');
  expectStats('one batch writes state and connection', '3 5 2 1');

  const invalid = { ...ingestState('m9', t1 + 4, 'invalid'), manufacturer: 'x/y' };
  ingest(invalid, ingestState('m1', t1 + 4, 'after-invalid'));
  expectStats('an invalid AGV name registers nothing', '3 5 2 1');
  expectEqual('an invalid message does not block the batch', stateColumn('e/5', 'order_id'), '"after-invalid"');
} finally {
  spacetime('delete', DB, ...BASE, '--yes');
}

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
console.log('all passed');
