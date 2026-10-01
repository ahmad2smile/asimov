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

// `error` is the latest error, or none.
const state = (agvName: string, error: unknown = NO_ERROR) =>
  call('upsert_agv_state', ...split(agvName), {
    order_id: '',
    last_node_id: '',
    error,
  });

const connection = (agvName: string, connectionState: string) =>
  call('upsert_agv_connection', ...split(agvName), { [connectionState]: [] });

// Runs `fn` and reports whether it failed with a "not found" error.
const expectNotFound = (label: string, fn: () => unknown) => {
  try {
    fn();
    report(false, label, 'expected an error, the call succeeded');
  } catch (e) {
    const out = String((e as { stderr?: unknown }).stderr ?? e);
    report(/not found/.test(out), label, `expected 'not found', got '${out}'`);
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
  expectNotFound('connection needs a registered AGV', () => connection('a/1', 'online'));
  expectNotFound('AGV needs a registered map', () => call('upsert_agv', 'a', '1', 'm1'));
  expectStats('failed calls change nothing', '0 0 0 0');

  register('a/1', 'm1');
  state('a/1');
  connection('a/1', 'online');
  expectStats('one online AGV', '1 1 1 0');

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
  check('search narrows the page', page('b/', { none: [] }, 5), '[1,[]]');
} finally {
  spacetime('delete', DB, ...BASE, '--yes');
}

if (failures > 0) {
  console.log(`${failures} failed`);
  process.exit(1);
}
console.log('all passed');
