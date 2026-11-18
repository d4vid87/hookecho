// Isolate browser worker scheduling from feed age and rendering, with identical real WASM/bytes.
// RUNS=10 CHROME_BIN=/usr/bin/chromium node scripts/web/freshness.mjs
import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import http from 'node:http';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {pageAssets, glueImports} from './shell-assets.mjs';

const origin = process.env.HOOKECHO_BASE_URL ?? 'https://app.hookecho.io/';
const output = process.env.HOOKECHO_FRESHNESS_OUTPUT ?? 'outputs/freshness-web';
const runs = Number(process.env.RUNS ?? 10);
assert(Number.isInteger(runs) && runs >= 1);
await fs.mkdir(output, {recursive: true});
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const assets = new Map();
const provenance = [];
async function download(path) {
  const response = await fetch(new URL(path, origin), {signal: AbortSignal.timeout(120000)});
  assert(response.ok, `${path}: ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  provenance.push({url: response.url, sha256: sha(bytes), bytes: bytes.length,
    date: response.headers.get('date'), age: response.headers.get('age'), etag: response.headers.get('etag')});
  assets.set(path.split('?')[0], bytes);
  return bytes;
}
const html = (await download('/')).toString();
const shell = pageAssets(html);
const glue = shell.find(path => /^\/dist\/hookecho-[a-f0-9]+\.js/.test(path));
const wasmPath = shell.find(path => path.endsWith('.wasm'));
const glueSource = await download(glue);
await Promise.all([download(wasmPath), ...glueImports(glueSource.toString()).map(download)]);
for (const file of ['decode-bridge.js', 'decode-worker.js']) {
  assets.set(`/baseline/${file}`, await download(`/${file}`));
  assets.set(`/candidate/${file}`, await fs.readFile(new URL(`../../web/${file}`, import.meta.url)));
}
const fixture = await fs.readFile(new URL('../../crates/wxdata/tests/data/kdmx-one-sweep.bin', import.meta.url));
assets.set('/fixture', fixture);
// Fault injection exercises recovery with real transferable buffers; it is not an observed trap.
const fault = '\nconst realHandler = self.onmessage; self.onmessage = e => e.data.op === "fault" ? self.postMessage({id:e.data.id, err:"injected trap", fatal:true}) : realHandler(e);';
for (const variant of ['baseline', 'candidate']) {
  assets.set(`/${variant}/fault-worker.js`, Buffer.from(assets.get(`/${variant}/decode-worker.js`).toString() + fault));
}
const report = {started: new Date().toISOString(), origin, runs, provenance,
  checkout: execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim(),
  fixture: {path: 'crates/wxdata/tests/data/kdmx-one-sweep.bin', bytes: fixture.length, sha256: sha(fixture)},
  js: Object.fromEntries(['baseline', 'candidate'].map(variant => [variant, Object.fromEntries(
    ['decode-bridge.js', 'decode-worker.js'].map(file => [file, sha(assets.get(`/${variant}/${file}`))]))])),
  limits: ['Historical KDMX fixture; no live-feed, observation-age, GPU/presentation, or physical-device claim.',
    'Cold means a new worker/heap with already compiled module; excludes network/download/compilation.',
    'Queue/round-trip use page monotonic time; decode is a subset of round-trip, not additive.',
    'Contended runs queue identical archive jobs, not a representative MRMS/satellite workload.',
    'Both variants use the same downloaded WASM and no application render loop.'], rows: [], failures: []};
const server = http.createServer((request, response) => {
  const path = new URL(request.url, 'http://localhost').pathname;
  const bytes = path === '/' ? Buffer.from('<!doctype html><title>HookEcho worker measurement</title>') : assets.get(path);
  response.writeHead(bytes ? 200 : 404, {'content-type': path.endsWith('.js') ? 'text/javascript'
    : path.endsWith('.wasm') ? 'application/wasm' : path === '/' ? 'text/html' : 'application/octet-stream'});
  response.end(bytes ?? 'missing');
});
await new Promise(resolve => server.listen(8767, '127.0.0.1', resolve));
const browser = await puppeteer.launch({headless: true, executablePath: process.env.CHROME_BIN ?? '/usr/bin/chromium', args: ['--no-sandbox']});
try {
  const page = await browser.newPage();
  await page.goto('http://127.0.0.1:8767/');
  report.browser = await browser.version();
  await page.evaluate(async (wasmPath, glue) => {
    window.module = await WebAssembly.compile(await (await fetch(wasmPath)).arrayBuffer());
    window.payload = new Uint8Array(await (await fetch('/fixture')).arrayBuffer());
    window.glue = new URL(glue, location.href).href;
    window.longTasks = [];
    new PerformanceObserver(list => window.longTasks.push(...list.getEntries().map(e => ({start: e.startTime, duration: e.duration}))))
      .observe({entryTypes: ['longtask']});
    window.measure = async (variant, mode, count) => {
      const {makeBridge} = await import(`/${variant}/decode-bridge.js`);
      const workers = [], dispatched = new Map(), received = new Map();
      const bridge = makeBridge({log: () => {}, spawnWorker: () => {
        const worker = new Worker(`/${variant}/${mode === 'failure' ? 'fault-worker' : 'decode-worker'}.js`, {type: 'module'});
        workers.push(worker);
        worker.postMessage({module: window.module, glue: window.glue});
        const post = worker.postMessage.bind(worker);
        worker.postMessage = (job, transfer) => {dispatched.set(job.id, performance.now()); return post(job, transfer);};
        worker.addEventListener('message', event => received.set(event.data.id, performance.now()));
        return worker;
      }});
      let id = 0;
      const run = async () => {
        const job = ++id, queued = performance.now();
        const bytes = await bridge(window.payload.slice(), 'decode', 'fixture:KDMX/kdmx-one-sweep.bin');
        const completed = performance.now();
        const digest = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(n => n.toString(16).padStart(2, '0')).join('');
        return {variant, mode, job, queueMs: dispatched.get(job) - queued,
          roundTripMs: received.get(job) - dispatched.get(job), totalMs: completed - queued,
          bytes: bytes.length, sha256: digest, timing: bridge.timings?.find(row => row.id === job) ?? null};
      };
      try {
        if (mode === 'failure') {
          const results = await Promise.allSettled([bridge(new Uint8Array([0]), 'fault'), bridge(window.payload.slice(), 'decode')]);
          return results.map(result => ({status: result.status, error: result.reason?.message ?? null, bytes: result.value?.length ?? null}));
        }
        if (mode !== 'cold-worker') await run();
        const rows = [];
        for (let i = 0; i < count; i++) {
          if (mode === 'contended') rows.push(...await Promise.all([run(), run()]));
          else rows.push(await run());
        }
        return rows;
      } finally { workers.forEach(worker => worker.terminate()); }
    };
  }, wasmPath, glue);
  for (let i = 0; i < runs; i++) for (const variant of ['baseline', 'candidate']) {
    report.rows.push(...await page.evaluate(variant => window.measure(variant, 'cold-worker', 1), variant));
  }
  for (const variant of ['baseline', 'candidate']) {
    for (const mode of ['warm-worker', 'contended']) report.rows.push(...await page.evaluate((variant, mode, runs) => window.measure(variant, mode, runs), variant, mode, runs));
    report.failures.push({variant, results: await page.evaluate(variant => window.measure(variant, 'failure', 1), variant)});
  }
  report.longTasks = await page.evaluate(() => window.longTasks);
  assert.equal(new Set(report.rows.map(row => row.sha256)).size, 1, 'scientific worker bytes changed');
  assert.equal(report.failures[1].results[1].status, 'fulfilled', 'candidate queued radar must survive worker replacement');
  report.summary = [];
  for (const variant of ['baseline', 'candidate']) for (const mode of ['cold-worker', 'warm-worker', 'contended']) {
    const rows = report.rows.filter(row => row.variant === variant && row.mode === mode);
    const summary = {variant, mode, n: rows.length};
    for (const metric of ['queueMs', 'roundTripMs', 'totalMs']) {
      const values = rows.map(row => row[metric]).sort((a, b) => a - b);
      summary[metric] = {p50: values[Math.ceil(values.length * .5) - 1], p95: values[Math.ceil(values.length * .95) - 1], max: values.at(-1)};
    }
    report.summary.push(summary);
  }
} finally {
  report.finished = new Date().toISOString();
  await fs.writeFile(`${output}/worker-report.json`, JSON.stringify(report, null, 2));
  await browser.close();
  server.close();
}
console.log(JSON.stringify({output, summary: report.summary, failures: report.failures}));
