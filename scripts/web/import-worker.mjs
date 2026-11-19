// Real compiled WASM in a module Worker; native fixture verification decodes returned CBOR.
// HOOKECHO_IMPORT_FIXTURES=/tmp/... cargo test -p hookecho --lib imports::tests::browser_worker_fixtures -- --ignored
// HOOKECHO_IMPORT_WASM=/tmp/bindgen-output HOOKECHO_IMPORT_FIXTURES=/tmp/... node scripts/web/import-worker.mjs
// HOOKECHO_IMPORT_VERIFY=1 HOOKECHO_IMPORT_FIXTURES=/tmp/... cargo test -p hookecho --lib imports::tests::browser_worker_fixtures -- --ignored
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';

const fixtures = path.resolve(process.env.HOOKECHO_IMPORT_FIXTURES);
const wasm = path.resolve(process.env.HOOKECHO_IMPORT_WASM);
const worker = await fs.readFile(new URL('../../web/decode-worker.js', import.meta.url));
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    if (req.method === 'POST' && /^\/results\/[a-z-]+\.cbor$/.test(url.pathname)) {
      const chunks = []; let size = 0;
      for await (const chunk of req) {
        size += chunk.length;
        if (size > 128 * 1024 * 1024) throw new Error('Result exceeds transfer budget');
        chunks.push(chunk);
      }
      await fs.writeFile(path.join(fixtures, path.basename(url.pathname)), Buffer.concat(chunks));
      res.end('ok'); return;
    }
    if (url.pathname === '/') { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><title>Import worker regression</title>'); return; }
    if (url.pathname === '/decode-worker.js') { res.setHeader('Content-Type', 'text/javascript'); res.end(worker); return; }
    const root = url.pathname.startsWith('/fixtures/') ? fixtures : wasm;
    const relative = url.pathname.replace(/^\/(?:fixtures\/)?/, '');
    const target = path.resolve(root, relative);
    if (!target.startsWith(root + path.sep)) throw new Error('Invalid fixture path');
    res.setHeader('Content-Type', target.endsWith('.wasm') ? 'application/wasm' : target.endsWith('.js') ? 'text/javascript' : 'application/octet-stream');
    res.end(await fs.readFile(target));
  } catch (error) { res.statusCode = 500; res.end(String(error)); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await puppeteer.launch({headless: true,
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), args: ['--no-sandbox']});
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const report = await page.evaluate(async () => {
    const module = await WebAssembly.compileStreaming(fetch('/hookecho_bg.wasm'));
    const worker = new Worker('/decode-worker.js', {type: 'module'});
    worker.postMessage({module, glue: new URL('/hookecho.js', location.href).href});
    const fixtures = await fetch('/fixtures/fixtures.json').then(r => r.json());
    const report = {};
    let id = 0;
    try {
      for (const fixture of fixtures) {
        const bytes = await fetch(`/fixtures/${fixture.name}.bin`).then(r => r.arrayBuffer());
        let ticks = 0, frames = 0, running = true;
        const timer = setInterval(() => ticks++, 1);
        const animate = () => { if (running) { frames++; requestAnimationFrame(animate); } };
        requestAnimationFrame(animate);
        const started = performance.now();
        let result;
        try {
          result = await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error(`Import ${fixture.name} timed out`)), 60_000);
            worker.onerror = event => { clearTimeout(timeout); reject(new Error(event.message)); };
            worker.onmessage = ({data}) => { clearTimeout(timeout); resolve(data); };
            worker.postMessage({id: ++id, op: 'import', bytes}, [bytes]);
          });
        } finally { running = false; clearInterval(timer); }
        report[fixture.name] = {expectedError: fixture.error, error: result.err ?? null, fatal: result.fatal ?? false,
          bytes: result.ok?.byteLength ?? 0, elapsedMs: performance.now()-started, ticks, frames};
        if (result.ok) {
          const stored = await fetch(`/results/${fixture.name}.cbor`, {method:'POST', body:result.ok});
          if (!stored.ok) throw new Error(await stored.text());
        }
      }
    } finally { worker.terminate(); }
    return report;
  });
  await fs.writeFile(path.join(fixtures, 'report.json'), JSON.stringify(report, null, 2));
  for (const [name, result] of Object.entries(report)) {
    assert.equal(Boolean(result.error), result.expectedError, name);
    assert.equal(result.fatal, false, name);
    if (!result.expectedError) assert(result.bytes > 0, name);
  }
  assert(report.gis.ticks > 0 && report.gis.frames > 0, 'Page timers and animation continue during large GIS worker preparation');
  assert.deepEqual(errors, []);
  console.log(JSON.stringify(report, null, 2));
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
