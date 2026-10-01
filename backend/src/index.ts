import spacetimedb from './schema';

export { default } from './schema';
export * from './agv';
export * from './fleet';

export const init = spacetimedb.init(_ctx => {
  // Called when the module is initially published
});

export const identityConnected = spacetimedb.clientConnected(_ctx => {
  // Called every time a new client connects
});

export const identityDisconnected = spacetimedb.clientDisconnected(_ctx => {
  // Called every time a client disconnects
});
