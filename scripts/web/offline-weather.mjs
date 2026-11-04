// Real IndexedDB regression tests against the Rust weather-pack implementation, without app UI.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import puppeteer from 'puppeteer';

const root = fileURLToPath(new URL('../../', import.meta.url));
const scratch = process.env.HOOKECHO_WEATHER_HARNESS ?? await fs.mkdtemp(path.join(os.tmpdir(), 'hookecho-weather-'));
const target = process.env.CARGO_TARGET_DIR ?? path.join(root, 'target');
const bindgen = process.env.WASM_BINDGEN ?? 'wasm-bindgen';
await fs.mkdir(path.join(scratch, 'src'), {recursive: true});
await fs.mkdir(path.join(scratch, 'pkg'), {recursive: true});
if (!process.env.HOOKECHO_WEATHER_SKIP_BUILD) {
  const cargo = await fs.readFile(path.join(root, 'Cargo.toml'), 'utf8');
  const profiles = cargo.slice(cargo.indexOf('[profile.release]'), cargo.indexOf('[workspace.package]'));
  await fs.writeFile(path.join(scratch, 'Cargo.toml'), `[package]
name = "hookecho-weather-audit"
version = "0.0.0"
edition = "2021"
[lib]
crate-type = ["cdylib"]
[dependencies]
hookecho = { path = ${JSON.stringify(path.join(root, 'crates/hookecho'))} }
wxdata = { path = ${JSON.stringify(path.join(root, 'crates/wxdata'))} }
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
serde_json = "1"
chrono = "0.4"
[workspace]
[patch.crates-io]
gribberish = { path = ${JSON.stringify(path.join(root, 'vendor/gribberish'))} }
nexrad-data = { path = ${JSON.stringify(path.join(root, 'vendor/nexrad-data'))} }
${profiles}`);
  await fs.copyFile(path.join(root, 'Cargo.lock'), path.join(scratch, 'Cargo.lock'));
  await fs.copyFile(new URL('./fixtures/offline-weather.rs', import.meta.url), path.join(scratch, 'src/lib.rs'));
  execFileSync('cargo', ['build', '--offline', '--profile', 'web', '--target', 'wasm32-unknown-unknown',
    '--manifest-path', path.join(scratch, 'Cargo.toml'), '--target-dir', target], {
    stdio: 'inherit',
    env: {...process.env, RUSTFLAGS: (process.env.RUSTFLAGS ?? '').includes('getrandom_backend')
      ? process.env.RUSTFLAGS : `${process.env.RUSTFLAGS ?? ''} --cfg getrandom_backend="wasm_js"`},
  });
  execFileSync(bindgen, [path.join(target, 'wasm32-unknown-unknown/web/hookecho_weather_audit.wasm'),
    '--target', 'web', '--out-dir', path.join(scratch, 'pkg')], {stdio: 'inherit'});
}
const server = http.createServer(async (request, response) => {
  const pathname = new URL(request.url, 'http://localhost').pathname;
  if (pathname === '/') { response.end('<!doctype html><title>Weather pack regression</title>'); return; }
  const name = path.resolve(scratch, 'pkg', `.${pathname}`);
  if (!name.startsWith(path.join(scratch, 'pkg') + path.sep)) { response.writeHead(403).end(); return; }
  try {
    response.setHeader('content-type', name.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
    response.end(await fs.readFile(name));
  } catch { response.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
let browser;
try {
  browser = await puppeteer.launch({executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true, args: ['--no-sandbox']});
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const checks = await page.evaluate(async () => {
    const api = await import('/hookecho_weather_audit.js');
    await api.default();
    const check = (value, message) => { if (!value) throw new Error(message); };
    const records = async (database, storeName) => {
      const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open(database);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      try {
        const store = db.transaction(storeName).objectStore(storeName);
        const requests = [store.getAllKeys(), store.getAll()];
        const values = await Promise.all(requests.map(request => new Promise((resolve, reject) => {
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        })));
        return Object.fromEntries(values[0].map((key, index) => [key, values[1][index]]));
      } finally { db.close(); }
    };
    const volumes = () => records('hookecho-packs', 'volumes');
    const metadata = async () => Object.fromEntries(Object.entries(await records('hookecho-objects', 'meta')).map(([key, value]) => [key, JSON.parse(value)]));
    const save = (date, names, satellites = []) => api.save_pack(date, JSON.stringify(names), JSON.stringify(satellites));
    for (const key of ['shared', 'old', 'new']) await api.seed_object(key);
    await save('2026-05-20', ['A', 'B'], ['shared', 'old']);
    await new Promise((resolve, reject) => {
      const open = indexedDB.open('hookecho-packs');
      open.onsuccess = () => {
        const db = open.result;
        const tx = db.transaction('volumes', 'readwrite');
        tx.objectStore('volumes').put(new Uint8Array([0]), 'legacy-orphan');
        tx.oncomplete = () => { db.close(); resolve(); };
        tx.onabort = () => { db.close(); reject(tx.error); };
      };
      open.onerror = () => reject(open.error);
    });
    await save('2026-05-21', ['B', 'D'], ['shared']);
    await save('2026-05-20', ['B', 'C'], ['new']);
    check(Object.keys(await volumes()).sort().join() === 'B,C,D', 'replacement leaked superseded volume A');
    let meta = await metadata();
    check(!meta.old.pinned && meta.shared.pinned && meta.new.pinned, 'replacement released shared pins or retained old pin');
    await api.delete_pack('2026-05-20');
    check(Object.keys(await volumes()).sort().join() === 'B,D', 'delete removed shared volume or retained unused volumes');
    meta = await metadata();
    check(!meta.new.pinned && meta.shared.pinned, 'delete broke shared weather ownership');

    await save('2026-05-20', ['A'], ['old']);
    const originalPut = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (...args) {
      const result = originalPut.apply(this, args);
      if (this.name === 'packs') this.transaction.abort(); // Quota-style abort after writes were queued.
      return result;
    };
    let failed = false;
    try { await save('2026-05-20', ['B'], ['new']); } catch { failed = true; }
    finally { IDBObjectStore.prototype.put = originalPut; }
    check(failed, 'injected transaction abort was reported as success');
    check(Object.keys(await volumes()).sort().join() === 'A,B,D', 'aborted replacement changed the old radar files');
    const manifests = await records('hookecho-packs', 'packs');
    check(JSON.parse(manifests['KTLX-2026-05-20']).volumes.join() === 'A', 'aborted replacement changed the manifest');
    meta = await metadata();
    check(meta.old.pinned && meta.shared.pinned && !meta.new.pinned, 'rollback changed old pins or leaked new pin');

    for (const mode of ['pin-abort', 'cancel']) {
      IDBObjectStore.prototype.put = function (...args) {
        const result = originalPut.apply(this, args);
        if (mode === 'pin-abort' && this.name === 'meta') {
          const item = JSON.parse(args[0]);
          if (item.key === 'new' && item.pinned) this.transaction.abort();
        }
        if (mode === 'cancel' && this.name === 'volumes') api.cancel_save();
        return result;
      };
      let rejected = false;
      try { await save('2026-05-20', ['B'], ['new']); } catch { rejected = true; }
      finally { IDBObjectStore.prototype.put = originalPut; }
      check(rejected, `${mode} was reported as a saved pack`);
      check(Object.keys(await volumes()).sort().join() === 'A,B,D', `${mode} changed previous radar files`);
      const state = await metadata();
      check(state.old.pinned && state.shared.pinned && !state.new.pinned, `${mode} changed old pins or leaked a new pin`);
    }

    let blobReads = 0;
    const originalGet = IDBObjectStore.prototype.get;
    IDBObjectStore.prototype.get = function (...args) {
      if (this.name === 'data') blobReads += 1;
      return originalGet.apply(this, args);
    };
    try { await save('2026-05-22', ['E']); }
    finally { IDBObjectStore.prototype.get = originalGet; }
    check(blobReads === 0, 'manifest accounting read existing satellite blobs');
    check(JSON.parse(await api.ready()).every(Boolean), 'readiness validation lost existing packs');
    await api.delete_pack('2026-05-20');
    await api.delete_pack('2026-05-21');
    await api.delete_pack('2026-05-22');
    check(Object.keys(await volumes()).length === 0, 'deleting all packs left orphaned radar files');
    check(Object.values(await metadata()).every(item => !item.pinned), 'deleting all packs left weather pins');
    const frame = document.createElement('iframe');
    frame.src = '/';
    await new Promise(resolve => { frame.onload = resolve; document.body.append(frame); });
    const other = await frame.contentWindow.eval(`(async () => {
      const api = await import('/hookecho_weather_audit.js'); await api.default(); return api;
    })()`);
    await Promise.all([save('2026-05-23', ['shared-volume', 'first']),
      other.save_pack('2026-05-24', JSON.stringify(['shared-volume', 'second']), '[]')]);
    check(Object.keys(await volumes()).sort().join() === 'first,second,shared-volume', 'concurrent contexts lost committed volumes');
    await api.delete_pack('2026-05-23');
    check(Object.keys(await volumes()).sort().join() === 'second,shared-volume', 'cross-context shared volume was deleted');
    await other.delete_pack('2026-05-24');
    check(Object.keys(await volumes()).length === 0, 'cross-context deletion retained orphan volumes');
    frame.remove();
    await save('2026-05-25', ['older']);
    await new Promise((resolve, reject) => {
      const open = indexedDB.open('hookecho-packs');
      open.onsuccess = () => {
        const db = open.result;
        const tx = db.transaction('packs', 'readwrite');
        const store = tx.objectStore('packs');
        const read = store.get('KTLX-2026-05-25');
        read.onsuccess = () => {
          const pack = JSON.parse(read.result);
          pack.bytes = 250 * 1024 * 1024;
          pack.saved_at = 9999999999; // A future/tied timestamp must not evict the new save first.
          store.put(JSON.stringify(pack), 'KTLX-2026-05-25');
        };
        tx.oncomplete = () => { db.close(); resolve(); };
        tx.onabort = () => { db.close(); reject(tx.error); };
      };
      open.onerror = () => reject(open.error);
    });
    await save('2026-05-26', ['newest']);
    check(Object.keys(await volumes()).join() === 'newest', 'budget eviction discarded the just-saved pack');
    await api.delete_pack('2026-05-26');
    return ['replacement/deletion preserve shared files and release unowned files, including legacy orphans',
      'aborted/cancelled save preserves old manifest/files and rolls back new pins',
      'manifest bookkeeping reads no existing weather blobs; readiness still verifies them',
      'concurrent browser contexts preserve committed and shared radar files',
      'budget eviction preserves the just-saved pack despite stale timestamps'];
  });
  assert.equal(checks.length, 5);
  console.log(JSON.stringify({checks}, null, 2));
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
  if (!process.env.HOOKECHO_WEATHER_HARNESS) await fs.rm(scratch, {recursive: true, force: true});
}
