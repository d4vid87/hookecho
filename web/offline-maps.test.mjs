import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const rust = await readFile(new URL('../crates/hookecho/src/offline.rs', import.meta.url), 'utf8');
const bridge = rust.match(/inline_js = r#"([\s\S]*?)"#/)[1].replaceAll('export ', '');

function storage({ available = 1e9, failWrite = false } = {}) {
  const entries = new Map();
  let persistent = false;
  const context = vm.createContext({
    Uint8Array, Response,
    navigator: { storage: {
      estimate: async () => ({ quota: available, usage: 0 }),
      persist: async () => { persistent = true; return true; },
    } },
    caches: {
      open: async () => ({
        match: async (key) => entries.get(key)?.clone(),
        put: async (key, response) => {
          if (failWrite) throw new Error('QuotaExceededError');
          entries.set(key, response);
        },
      }),
      delete: async () => { entries.clear(); },
    },
  });
  vm.runInContext(bridge, context);
  return { context, entries, persistent: () => persistent };
}

test('saved tiles can be read without fetch, and missing tiles remain missing', async () => {
  const { context, persistent } = storage();
  await context.offlinePrepare(300);
  assert.equal(persistent(), true);
  await context.offlineWrite('https://app.test/proxy/tiles/8/3/4.pbf', new Uint8Array([1, 2, 3]));
  assert.deepEqual(await context.offlineRead('https://app.test/proxy/tiles/8/3/4.pbf'), new Uint8Array([1, 2, 3]));
  assert.equal(await context.offlineRead('https://app.test/proxy/tiles/8/3/5.pbf'), null);
});

test('quota preflight leaves the previous saved cache intact', async () => {
  const { context, entries } = storage({ available: 20 });
  entries.set('existing', new Response('saved'));
  await assert.rejects(context.offlinePrepare(21), /Not enough browser storage/);
  assert.equal(entries.size, 1);
});

test('a failed persistent write rejects instead of reporting success', async () => {
  const { context } = storage({ failWrite: true });
  await assert.rejects(context.offlineWrite('https://app.test/tile', new Uint8Array([1])), /QuotaExceededError/);
});

test('corrupt tiles are removed from map caches without touching the app shell', async () => {
  const removed = [];
  const context = vm.createContext({
    caches: {
      keys: async () => ['hookecho-offline-maps-v1', 'tiles-v2', 'shell-current'],
      open: async (name) => ({ delete: async (url) => removed.push([name, url]) }),
    },
  });
  vm.runInContext(bridge, context);
  await context.offlineDelete('https://app.test/proxy/tile');
  assert.deepEqual(removed, [
    ['hookecho-offline-maps-v1', 'https://app.test/proxy/tile'],
    ['tiles-v2', 'https://app.test/proxy/tile'],
  ]);
});

test('deployment cleanup preserves explicitly saved map tiles', async () => {
  const source = await readFile(new URL('./sw.src.js', import.meta.url), 'utf8');
  const kept = 'hookecho-offline-maps-v1';
  const listeners = {};
  const deleted = [];
  const context = vm.createContext({
    __SHELL__: [], __VERSION__: 'test',
    caches: { keys: async () => [kept, 'tiles-v2', 'shell-test', 'shell-old'], delete: async (name) => deleted.push(name) },
    self: { addEventListener: (event, callback) => listeners[event] = callback, clients: { claim() {} } },
  });
  vm.runInContext(source, context);
  await new Promise((resolve, reject) => listeners.activate({ waitUntil: (promise) => promise.then(resolve, reject) }));
  assert.deepEqual(deleted, ['shell-old']);
});
