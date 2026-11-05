import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const source = await readFile(new URL('./sw.src.js', import.meta.url), 'utf8');
const html = '<html><script src="./dist/hookecho-12345678.js?boot=2"></script></html>';
const page = (body = html, status = 200) => new Response(body, {status, headers: {'content-type': 'text/html'}});
function worker(fetch) {
  const listeners = {};
  const stores = new Map();
  const key = value => typeof value === 'string' ? value : value.url;
  const caches = {
    open: async name => {
      if (!stores.has(name)) stores.set(name, new Map());
      const store = stores.get(name);
      return {
        match: async request => store.get(key(request))?.clone(),
        put: async (request, response) => store.set(key(request), response.clone()),
        keys: async () => [...store.keys()].map(url => ({url})),
        delete: async request => store.delete(key(request)),
        addAll: async paths => {
          for (const path of paths) {
            const response = await context.fetch(path);
            if (!response.ok) throw new Error('Unsuccessful precache fetch');
            store.set(path, response.clone());
          }
        },
      };
    },
    match: async request => {
      for (const store of stores.values()) {
        const hit = store.get(key(request));
        if (hit) return hit.clone();
      }
    },
    delete: async name => stores.delete(name),
  };
  const context = vm.createContext({
    __SHELL__: ['/', '/dist/hookecho-12345678.js?boot=2'], __VERSION__: 'test',
    caches, fetch, Response, Headers, URL,
    self: {location: new URL('https://app.test/'), addEventListener: (name, callback) => listeners[name] = callback},
  });
  vm.runInContext(source, context);
  return {
    context, caches, stores,
    install: () => new Promise((resolve, reject) => listeners.install({waitUntil: task => task.then(resolve, reject)})),
    navigate: () => new Promise(resolve => listeners.fetch({
      request: {url: 'https://app.test/', method: 'GET', mode: 'navigate'}, respondWith: task => resolve(task),
    })),
    tile: (id = 1) => new Promise(resolve => listeners.fetch({
      request: {url: `https://app.test/tiles/1/2/${id}.png`, method: 'GET'}, respondWith: resolve,
    })),
  };
}

test('503 and network failures preserve the installed offline app', async () => {
  const w = worker(async () => page());
  await w.install();
  w.context.fetch = async () => page('temporary outage', 503);
  assert.equal(await (await w.navigate()).text(), html);
  w.context.fetch = async () => { throw new Error('offline'); };
  assert.equal(await (await w.navigate()).text(), html);
  assert.equal(await (await (await w.caches.open('shell-test')).match('/')).text(), html);
});

test('a newer online index never replaces this version of the offline shell', async () => {
  const w = worker(async () => page());
  await w.install();
  w.context.fetch = async () => page('<html>newer app with other assets</html>');
  assert.match(await (await w.navigate()).text(), /newer app/);
  w.context.fetch = async () => { throw new Error('offline'); };
  assert.equal(await (await w.navigate()).text(), html);
});

test('an install racing a deployment rejects its mixed shell and leaves the old one intact', async () => {
  const w = worker(async () => page('<html>different bundle</html>'));
  const old = await w.caches.open('shell-old');
  await old.put('/', page('older working app'));
  await assert.rejects(w.install(), /changed during installation/);
  assert.equal(w.stores.has('shell-test'), false);
  assert.equal(await (await old.match('/')).text(), 'older working app');
});

test('install waitUntil includes all precache writes', async () => {
  let complete;
  const pending = new Promise(resolve => complete = resolve);
  const w = worker(async () => { await pending; return page(); });
  let installed = false;
  const install = w.install().then(() => { installed = true; });
  await Promise.resolve();
  assert.equal(installed, false);
  complete();
  await install;
  assert.equal(w.stores.get('shell-test').size, 2);
});

async function finishWrites(w) {
  for (let i = 0; i < 100 && vm.runInContext('pendingTileWrites', w.context); i++) {
    await new Promise(resolve => setTimeout(resolve, 5));
  }
  assert.equal(vm.runInContext('pendingTileWrites', w.context), 0);
}

test('slow storage admits a bounded number of copies without delaying map responses', async () => {
  let copies = 0, release;
  const stalled = new Promise(resolve => release = resolve);
  const w = worker(async () => {
    const response = new Response('tile');
    const clone = response.clone.bind(response);
    response.clone = () => { copies++; return clone(); };
    return response;
  });
  const open = w.caches.open;
  w.caches.open = async name => { await stalled; return open(name); };
  const displayed = await Promise.all(Array.from({length: 100}, (_, id) => w.tile(id).then(r => r.text())));
  assert.ok(displayed.every(body => body === 'tile'));
  assert.equal(copies, 8);
  assert.equal(vm.runInContext('pendingTileWrites', w.context), 8);
  release();
  await finishWrites(w);
  assert.equal(w.stores.get('tiles-v2').size, 8);
  await (await w.tile(200)).text();
  await finishWrites(w);
  assert.equal(w.stores.get('tiles-v2').size, 9);
});

test('oversized bodies with and without content-length are displayed but not cached', async () => {
  const bytes = new Uint8Array(8 * 1024 * 1024 + 1);
  for (const headers of [{}, {'content-length': String(bytes.length)}]) {
    const w = worker(async () => new Response(bytes, {headers}));
    assert.equal((await (await w.tile()).arrayBuffer()).byteLength, bytes.length);
    await finishWrites(w);
    assert.equal(w.stores.get('tiles-v2')?.size ?? 0, 0);
  }
});

test('a failed cache write releases its slot and later tiles can be cached', async () => {
  const w = worker(async () => new Response('tile'));
  const open = w.caches.open;
  let fail = true;
  w.caches.open = async name => {
    if (fail) throw new Error('storage unavailable');
    return open(name);
  };
  assert.equal(await (await w.tile()).text(), 'tile');
  await finishWrites(w);
  fail = false;
  await (await w.tile(2)).text();
  await finishWrites(w);
  assert.equal(w.stores.get('tiles-v2').size, 1);
});

test('partial tile responses cannot poison the complete-tile cache', async () => {
  const w = worker(async () => new Response('part', {status: 206, headers: {'content-range': 'bytes 0-3/8'}}));
  const response = await w.tile();
  assert.equal(response.status, 206);
  assert.equal(await response.text(), 'part');
  await finishWrites(w);
  assert.equal(w.stores.get('tiles-v2')?.size ?? 0, 0);
});
