import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('./amy-bootstrap.js', import.meta.url), 'utf8')
  .replace('export function prepareAmy', 'function prepareAmy');
const version = 'amy-medium-39ab474b';
const modelName = 'en_US-amy-medium.onnx';
const modelBytes = 63201294;
const hash = 'b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18';
const parts = [20971520, 20971520, 20971520, 286734];
const chunkUrl = i => `/voice/${version}/model.part-0${i}`;

// Model responses stream small chunks. Reading a whole part into another ArrayBuffer fails the
// test, and OPFS stores metadata only: these tests need one real model assembly, not disk copies.
function harness({ backend = { cache: new Map(), files: new Map(), clock: 1 }, autoReady = true } = {}) {
  const stats = { requests: [], writes: [], hashes: 0, modelAllocations: 0, fileReads: [] };
  const workers = [], timers = new Map();
  let badHash = false, oversized = false;
  function descriptor(url) {
    const part = /model\.part-0([0-3])$/.exec(url);
    if (part) return { bytes: parts[Number(part[1])] + (oversized && part[1] === '3' ? 1 : 0), chunk: true };
    if (url.endsWith('/manifest.json')) return { json: { voice: 'en_US-amy-medium', chunks: parts.map((_, i) => `model.part-0${i}`) } };
    return { text: url.endsWith('.onnx.json') ? '{"sample_rate":22050}' : 'runtime asset' };
  }
  function response(desc) {
    return {
      ok: true, status: 200, clone: () => response(desc),
      json: async () => desc.json,
      arrayBuffer: async () => {
        assert.ok(!desc.chunk, 'model parts must be streamed, not copied into a second full buffer');
        return new TextEncoder().encode(desc.text).buffer;
      },
      body: { getReader() {
        let offset = 0;
        return {
          async read() {
            if (offset >= desc.bytes) return { done: true };
            const value = new Uint8Array(Math.min(65536, desc.bytes - offset)).fill(1);
            offset += value.length;
            return { done: false, value };
          },
          async cancel() {}, releaseLock() {},
        };
      } },
      descriptor: desc,
    };
  }
  const fetch = async url => { stats.requests.push(url); return response(descriptor(url)); };
  const cache = {
    async match(url) { return backend.cache.has(url) ? response(backend.cache.get(url)) : undefined; },
    async add(url) { backend.cache.set(url, (await fetch(url)).descriptor); },
    async put(url, res) { backend.cache.set(url, res.descriptor); },
    async delete(url) { return backend.cache.delete(url); },
  };
  const dir = {
    async getFileHandle(name, options) {
      if (!backend.files.has(name)) {
        if (!options?.create) throw new Error('NotFound');
        backend.files.set(name, { size: 0, lastModified: backend.clock++, text: '' });
      }
      return {
        async getFile() {
          stats.fileReads.push(name);
          const file = backend.files.get(name);
          return { ...file, text: async () => file.text,
            arrayBuffer: async () => { throw new Error('Warm OPFS model must not be reread'); } };
        },
        async createWritable() {
          let data;
          return {
            async write(value) { data = value; },
            async close() {
              stats.writes.push(name);
              backend.files.set(name, { size: typeof data === 'string' ? Buffer.byteLength(data) : data.byteLength,
                lastModified: backend.clock++, text: typeof data === 'string' ? data : '' });
            },
            async abort() {},
          };
        },
      };
    },
    async removeEntry(name) { backend.files.delete(name); },
  };
  class FakeWorker {
    constructor(url) { this.url = url; this.terminated = false; this.messages = []; workers.push(this); }
    postMessage(message) {
      this.messages.push(message);
      if (message.type === 'prepare' && autoReady) queueMicrotask(() => this.emit({ type: 'ready' }));
    }
    emit(data) { this.onmessage?.({ data }); }
    terminate() { this.terminated = true; }
  }
  const context = vm.createContext({
    document: { documentElement: { dataset: {} } },
    caches: { open: async name => { assert.equal(name, `voice-${version}`); return cache; } },
    navigator: { storage: { getDirectory: async () => ({ getDirectoryHandle: async () => dir }) } },
    fetch, Worker: FakeWorker,
    crypto: { subtle: { digest: async (_, data) => {
      stats.hashes++;
      assert.equal(data.byteLength, modelBytes);
      assert.equal(data[0], 1); assert.equal(data[data.length - 1], 1);
      return Uint8Array.from(Buffer.from(badHash ? '00'.repeat(32) : hash, 'hex')).buffer;
    } } },
    Uint8Array: new Proxy(Uint8Array, { construct(target, args) {
      if (args[0] === modelBytes) stats.modelAllocations++;
      return Reflect.construct(target, args);
    } }),
    setTimeout(fn) { const id = Symbol(); timers.set(id, fn); return id; },
    clearTimeout(id) { timers.delete(id); },
    URL, Blob, console,
  });
  vm.runInContext(source, context);
  return { context, stats, workers, timers, backend,
    prepare: () => vm.runInContext('prepareAmy()', context),
    retry: () => context.__hookechoAmyRetry(),
    setBadHash(value) { badHash = value; }, setOversized(value) { oversized = value; },
  };
}

async function untilWorker(env, count = 1) {
  for (let i = 0; i < 20 && env.workers.length < count; i++) await new Promise(setImmediate);
  assert.equal(env.workers.length, count);
  return env.workers.at(-1);
}

test('import stays idle; enable streams one model buffer and waits for worker readiness', async () => {
  const env = harness({ autoReady: false });
  assert.deepEqual(env.stats.requests, []);
  assert.deepEqual(env.stats.fileReads, []);
  assert.equal(env.workers.length, 0);
  const pending = env.prepare();
  assert.equal(env.prepare(), pending);
  assert.equal(env.retry(), pending, 'retry while preparing must not start overlapping work');
  let completed = false;
  pending.then(() => { completed = true; });
  const worker = await untilWorker(env);
  assert.equal(completed, false);
  assert.match(env.context.__hookechoAmyStatus, /starting voice/);
  assert.equal(env.stats.modelAllocations, 1);
  assert.equal(env.stats.hashes, 1);
  assert.deepEqual(env.stats.writes, [modelName, `${modelName}.json`, 'hookecho-amy-verified.json']);
  assert.equal(env.context.__hookechoAmyModel, undefined);
  assert.equal(worker.url, '/amy-worker.js?protocol=2');
  worker.emit({ type: 'ready' });
  assert.equal(await pending, true);
  assert.equal(env.context.__hookechoAmyStatus, 'Amy — ready');
  assert.equal(env.timers.size, 0);
  assert.equal(await env.prepare(), true);
  assert.equal(env.workers.length, 1);

  const warm = harness({ backend: env.backend });
  assert.equal(await warm.prepare(), true);
  assert.deepEqual(warm.stats.requests, [], 'warm startup must not redownload present runtime assets or manifest');
  assert.equal(warm.stats.hashes, 0);
  assert.equal(warm.stats.modelAllocations, 0);
  assert.deepEqual(warm.stats.writes, []);
  assert.ok(warm.stats.fileReads.includes(modelName));

  warm.backend.cache.delete('/voice/runtime/ort-wasm.wasm');
  const missingRuntime = harness({ backend: warm.backend });
  assert.equal(await missingRuntime.prepare(), true);
  assert.deepEqual(missingRuntime.stats.requests, ['/voice/runtime/ort-wasm.wasm']);
  assert.equal(missingRuntime.stats.modelAllocations, 0);
});

test('changed OPFS metadata invalidates the verified marker and repairs from cached chunks', async () => {
  const original = harness();
  assert.equal(await original.prepare(), true);
  original.backend.files.get(modelName).lastModified++;
  const repaired = harness({ backend: original.backend });
  assert.equal(await repaired.prepare(), true);
  assert.equal(repaired.stats.hashes, 1);
  assert.equal(repaired.stats.modelAllocations, 1);
  assert.deepEqual(repaired.stats.requests, [`/voice/${version}/manifest.json`]);
  assert.equal(repaired.stats.writes.length, 3);
});

test('prepare errors without a speech job id fail promptly; retry reuses installed assets', async () => {
  const env = harness({ autoReady: false });
  const pending = env.prepare();
  const failed = await untilWorker(env);
  failed.emit({ type: 'error', phase: 'prepare', message: 'WASM initialization failed' });
  assert.equal(await pending, false);
  assert.match(env.context.__hookechoAmyStatus, /unavailable.*WASM initialization failed/);
  assert.equal(failed.terminated, true);
  assert.equal(env.timers.size, 0);
  const retry = env.retry();
  assert.equal(env.retry(), retry);
  const second = await untilWorker(env, 2);
  failed.emit({ type: 'ready' });
  assert.match(env.context.__hookechoAmyStatus, /starting voice/, 'late messages cannot revive a terminated worker');
  second.emit({ type: 'ready' });
  assert.equal(await retry, true);
  assert.equal(env.stats.hashes, 1);
  assert.equal(env.stats.writes.length, 3);
});

test('worker startup crashes, unreadable messages, and deadline all settle and permit retry', async () => {
  const seed = harness();
  assert.equal(await seed.prepare(), true);
  for (const failure of ['crash', 'message', 'deadline']) {
    const env = harness({ backend: seed.backend, autoReady: false });
    const pending = env.prepare();
    const worker = await untilWorker(env);
    if (failure === 'crash') worker.onerror({ message: 'worker crashed', preventDefault() {} });
    if (failure === 'message') worker.onmessageerror();
    if (failure === 'deadline') [...env.timers.values()][0]();
    assert.equal(await pending, false, failure);
    assert.equal(worker.terminated, true, failure);
    assert.match(env.context.__hookechoAmyStatus, /unavailable/);
    const retry = env.retry();
    (await untilWorker(env, 2)).emit({ type: 'ready' });
    assert.equal(await retry, true, failure);
    assert.equal(env.stats.modelAllocations, 0);
  }
});

test('corrupt or oversized model chunks never reach OPFS and are removed before retry', async () => {
  for (const problem of ['checksum', 'oversized']) {
    const env = harness();
    env.setBadHash(problem === 'checksum'); env.setOversized(problem === 'oversized');
    assert.equal(await env.prepare(), false, problem);
    assert.deepEqual(env.stats.writes, []);
    assert.equal(env.workers.length, 0);
    assert.ok(parts.every((_, i) => !env.backend.cache.has(chunkUrl(i))));
    assert.match(env.context.__hookechoAmyStatus, /unavailable/);
    env.setBadHash(false); env.setOversized(false);
    assert.equal(await env.retry(), true, problem);
    assert.equal(env.stats.writes.length, 3);
  }
});

test('page boot imports voice controls without enabling voice downloads', () => {
  const index = readFileSync(new URL('./index.src.html', import.meta.url), 'utf8');
  assert.match(index, /import ["']\.\/amy-bootstrap\.js["']/);
  assert.doesNotMatch(index, /prepareAmy\s*\(/);
});

const runtimeFile = new URL('./voice/runtime/ort.wasm.min.js', import.meta.url);
test('provisioned ONNX browser bundle exposes the module API used by the voice worker',
  { skip: !existsSync(runtimeFile) && 'Run scripts/web/piper-runtime.sh to provision runtime assets' }, async () => {
    const module = await import(`data:text/javascript;base64,${readFileSync(runtimeFile).toString('base64')}`);
    assert.equal(typeof module.default?.InferenceSession?.create, 'function');
    assert.equal(typeof module.default?.Tensor, 'function');
  });
