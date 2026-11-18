// Same historical one-pane fixture and local HTTP transport for cold/warm app measurements.
// HOOKECHO_BASE_URL=https://app.hookecho.io/ mirrors that shell on localhost:8767 first.
// HOOKECHO_BASE_URL=http://127.0.0.1:8766/ measures an existing local candidate server.
// HOOKECHO_COMPARE_BASE_URL enables alternating local baseline/candidate cold+warm pairs.
import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {shellAssets} from './shell-assets.mjs';
import {fixtureRadar} from './fixtures/radar.mjs';

const source = process.env.HOOKECHO_BASE_URL ?? 'https://app.hookecho.io/';
const compare = process.env.HOOKECHO_COMPARE_BASE_URL;
if (compare) {
  assert([source, compare].every(url => ['127.0.0.1', 'localhost'].includes(new URL(url).hostname)),
    'paired measurements require two locally hosted shells');
}
const output = process.env.HOOKECHO_FRESHNESS_OUTPUT ?? 'outputs/freshness-web/app-baseline';
const runs = Number(process.env.RUNS ?? 10);
assert(Number.isInteger(runs) && runs > 0);
await fs.mkdir(output, {recursive: true});
const report = {started: new Date().toISOString(), source, runs, viewport: [1200, 900],
  comparison: compare ? {baseline: source, candidate: compare, order: 'baseline first on even trials; candidate first on odd trials'} : null,
  fixture: 'Committed historical KDMX single sweep under synthetic current-date names; one pane, ArchiveOnly, empty vector tiles/overlays/NWS status.',
  environment: {platform: os.platform(), release: os.release(), cpu: os.cpus()[0]?.model,
    logicalCpus: os.cpus().length, memoryBytes: os.totalmem(), node: process.version},
  limits: ['First decode/bin completion and map preparation are not physical presentation timestamps.',
    'Cold is a fresh browser context; warm is a reload in that context. HTTP cache is allowed, not assumed hit; inspect resource transfer sizes. Service worker bypassed.',
    'Remote shell is mirrored locally, uncompressed; these timings exclude real CDN/mobile network conditions.',
    'Chromium headless SwiftShader, not hardware GPU, native desktop, Android or a live-feed comparison.',
    'Input latency is wheel event to first changed map camera record, not monitor presentation.'],
  provenance: [], rows: [], errors: []};
let server, base = source;
if (new URL(source).hostname !== '127.0.0.1' && new URL(source).hostname !== 'localhost') {
  const files = new Map();
  async function download(path) {
    const response = await fetch(new URL(path, source), {signal: AbortSignal.timeout(120000)});
    assert(response.ok, `${path}: ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    const headers = {'content-type': response.headers.get('content-type') ?? 'application/octet-stream',
      'cache-control': response.headers.get('cache-control') ?? 'no-cache'};
    files.set(path.split('?')[0], {bytes, headers});
    report.provenance.push({url: response.url, sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length, headers});
    return bytes.toString();
  }
  const html = await download('/');
  const paths = await shellAssets(html, download);
  await Promise.all(paths.filter(path => !files.has(path.split('?')[0])).map(download));
  server = http.createServer((request, response) => {
    const file = files.get(new URL(request.url, 'http://localhost').pathname);
    response.writeHead(file ? 200 : 404, file?.headers ?? {});
    response.end(file?.bytes ?? 'not mirrored');
  });
  await new Promise(resolve => server.listen(8767, '127.0.0.1', resolve));
  base = 'http://127.0.0.1:8767/';
}
const browser = await puppeteer.launch({headless: true, executablePath: process.env.CHROME_BIN ?? '/usr/bin/chromium',
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
try {
  report.browser = await browser.version();
  for (let trial = 0; trial < runs; trial++) {
    const variants = compare ? [{name: 'baseline', url: source}, {name: 'candidate', url: compare}] : [{name: 'single', url: base}];
    if (compare && trial % 2) variants.reverse();
    for (const variant of variants) {
    base = variant.url;
    const context = await browser.createBrowserContext();
    const page = await context.newPage();
    await page.setViewport({width: 1200, height: 900, deviceScaleFactor: 1});
    await page.setBypassServiceWorker(true);
    await fixtureRadar(page, {vector: true, emptyOverlays: true, vectorResponse: async () => ({
      status: 200, contentType: 'application/json',
      body: JSON.stringify({tiles: [`${new URL(base).origin}/vector-fixture/{z}/{x}/{y}.pbf`]}),
    }), alertsResponse: async () => ({
      status: 200, contentType: 'application/geo+json', body: '{"type":"FeatureCollection","features":[]}',
    })});
    page.on('pageerror', error => report.errors.push({trial, message: error.message}));
    await page.evaluateOnNewDocument(() => {
      Object.defineProperty(navigator, 'gpu', {value: undefined});
      localStorage.clear();
      localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true, seeded_workspaces: true,
        default_site: 'KDMX', radar_feed: 'ArchiveOnly', live_loop_frames: 1}));
      globalThis.__hookechoMapTraceEnabled = true;
      window.measurement = {jobs: [], longTasks: [], inputs: [], diagnostics: []};
      new PerformanceObserver(list => measurement.longTasks.push(...list.getEntries().map(e => ({start: e.startTime, duration: e.duration}))))
        .observe({entryTypes: ['longtask']});
      addEventListener('wheel', () => measurement.inputs.push({at: performance.now(), zoom: globalThis.__hookechoMapTrace?.frames.at(-1)?.[3]}), {capture: true, passive: true});
      const create = URL.createObjectURL.bind(URL);
      URL.createObjectURL = blob => {blob.text().then(text => {try {const value = JSON.parse(text); if (value.schema === 'hookecho.diagnostics/v2') measurement.diagnostics.push(value);} catch {}}); return create(blob);};
      let bridge;
      Object.defineProperty(globalThis, '__decodeVolume', {configurable: true, get: () => bridge, set: value => {
        bridge = (bytes, op, context) => {
          const start = performance.now();
          const promise = value(bytes, op, context);
          promise.then(() => measurement.jobs.push({op, context, start, end: performance.now(), status: 'ok'}),
            error => measurement.jobs.push({op, context, start, end: performance.now(), status: error.message}));
          return promise; // Keep the original cancel hook and transferable-buffer ownership.
        };
        bridge.timings = value.timings;
      }});
    });
    const cdp = await page.createCDPSession();
    await cdp.send('Performance.enable');
    for (const mode of ['cold', 'warm']) {
      const start = performance.now();
      if (mode === 'cold') await page.goto(base + '#goto=KDMX,-93.72,41.73,7', {waitUntil: 'domcontentloaded', timeout: 90000});
      else await Promise.all([
        page.waitForNavigation({waitUntil: 'domcontentloaded', timeout: 90000}),
        page.evaluate(url => { history.replaceState(null, '', url); location.reload(); }, base + '#goto=KDMX,-93.72,41.73,7'),
      ]);
      await page.waitForFunction(() => !document.getElementById('boot') && measurement.jobs.some(job => job.op === 'bin' && job.status === 'ok'), {timeout: 90000});
      const usablePreparationWallMs = performance.now() - start;
      await wait(750);
      const initialCamera = await page.evaluate(() => globalThis.__hookechoMapTrace?.frames.at(-1)?.slice(3, 6));
      assert(initialCamera && Math.abs(initialCamera[0] - 7) < 1e-6, 'every trial must start at zoom 7');
      assert(Math.abs(initialCamera[1] - (-93.72 + 180) / 360) < 1e-8, 'every trial must use the same longitude');
      assert(Math.abs(initialCamera[2] - (1 - Math.log(Math.tan(Math.PI / 4 + 41.73 * Math.PI / 360)) / Math.PI) / 2) < 1e-8, 'every trial must use the same latitude');
      for (let i = 0; i < 12; i++) {
        await page.mouse.move(600, 400);
        await page.mouse.wheel({deltaY: i < 6 ? -24 : 24});
        await wait(50);
      }
      await wait(500);
      const metrics = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(metric => [metric.name, metric.value]));
      const row = await page.evaluate(() => {
        const frames = globalThis.__hookechoMapTrace?.frames ?? [];
        return { ...measurement, mapTrace: globalThis.__hookechoMapTrace ?? null,
          workerTimings: globalThis.__decodeVolume?.timings ?? null,
          firstDecodeMs: measurement.jobs.find(job => job.op === 'decode' && job.status === 'ok')?.end ?? null,
          firstBinMs: measurement.jobs.find(job => job.op === 'bin' && job.status === 'ok')?.end ?? null,
          inputToChangedMapMs: measurement.inputs.map(input => {
            const frame = frames.find(frame => frame[0] >= input.at && frame[3] !== input.zoom);
            return frame ? frame[0] - input.at : null;
          }), resources: performance.getEntriesByType('resource').map(e => ({name: e.name, duration: e.duration,
            transferSize: e.transferSize, encodedBodySize: e.encodedBodySize, decodedBodySize: e.decodedBodySize})) };
      });
      assert(row.mapTrace?.frames.length > 0, 'map preparation trace missing');
      assert.equal(row.inputs.length, 12, 'wheel input missing');
      report.rows.push({trial, variant: variant.name, mode, initialCamera, usablePreparationWallMs, loadAverage: os.loadavg(), metrics, ...row});
      if (trial === 0) await page.screenshot({path: `${output}/${compare ? variant.name + '-' : ''}${mode}.png`});
      await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
      console.log(JSON.stringify({trial, variant: variant.name, mode, firstDecodeMs: row.firstDecodeMs, firstBinMs: row.firstBinMs, heapBytes: metrics.JSHeapUsedSize}));
    }
    if (trial === runs - 1) {
      await page.mouse.click(600, 300);
      await page.keyboard.down('Control'); await page.keyboard.press('K'); await page.keyboard.up('Control');
      await wait(400); await page.keyboard.type('Save diagnostics'); await wait(500); await page.keyboard.press('Enter');
      try {
        await page.waitForFunction(() => measurement.diagnostics.length > 0, {timeout: 15000});
        report.diagnostics = await page.evaluate(() => measurement.diagnostics.at(-1));
      } catch (error) { report.errors.push({stage: 'diagnostics export', message: error.message}); }
      const hasTrace = await page.evaluate(async () => {
        const glue = performance.getEntriesByType('resource').find(e => /\/dist\/hookecho-[a-f0-9]+\.js/.test(e.name))?.name;
        const wasm = glue ? await import(glue) : null;
        window.freshnessTrace = typeof wasm?.radar_trace === 'function' ? () => JSON.parse(wasm.radar_trace()) : null;
        return !!window.freshnessTrace;
      });
      if (process.env.EXPECT_TRACE === '1' && (!compare || variant.name === 'candidate')) assert(hasTrace, 'candidate radar trace export missing');
      if (hasTrace) {
        const before = await page.evaluate(() => {
          const count = freshnessTrace().filter(event => event.stage === 'network_recovered').length;
          window.dispatchEvent(new Event('online'));
          return count;
        });
        await page.waitForFunction(before => freshnessTrace().filter(event => event.stage === 'network_recovered').length === before + 1, {timeout: 10000}, before);
        await wait(750);
        report.radarTrace = await page.evaluate(() => freshnessTrace());
        assert.equal(report.radarTrace.filter(event => event.stage === 'network_recovered').length, before + 1, 'online event must be consumed once');
        const paints = report.radarTrace.filter(event => event.stage === 'paint_recorded');
        assert(paints.length > 0, 'candidate must acknowledge a radar map paint');
        assert.equal(new Set(paints.map(event => `${event.details.pane}:${event.details.token}`)).size, paints.length, 'unchanged frames must not duplicate paint acknowledgements');
        report.paintedFrames = paints.map(paint => {
          const upload = report.radarTrace.find(event => event.stage === 'upload_requested' && event.details.token === paint.details.token);
          assert(upload, 'painted token must have exact upload metadata');
          assert.equal(paint.details.pane, 0);
          assert.equal(upload.details.site, 'KDMX');
          assert.equal(upload.details.moment, 'Reflectivity');
          assert.match(upload.details.time_basis, /Selected sweep/);
          assert(upload.details.oldest_radial_ms > 0 && upload.details.oldest_radial_ms <= upload.details.newest_radial_ms);
          assert.equal(Date.parse(upload.details.display_time), upload.details.newest_radial_ms);
          assert.equal(paint.details.actual_presentation_utc_ms, null);
          return {paint, upload};
        });
        report.onlineEventConsumedOnce = true;
      } else report.radarTrace = null;
    }
    await context.close();
    }
  }
  assert.deepEqual(report.errors, [], 'page and diagnostics errors invalidate the app benchmark');
  const distribution = values => {
    const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
    return {n: sorted.length, p50: sorted[Math.ceil(sorted.length * .5) - 1] ?? null,
      p95: sorted[Math.ceil(sorted.length * .95) - 1] ?? null, max: sorted.at(-1) ?? null};
  };
  assert.equal(distribution([3, 1, 2]).p50, 2);
  report.summary = [...new Set(report.rows.map(row => row.variant))].flatMap(variant => ['cold', 'warm'].map(mode => {
    const rows = report.rows.filter(row => row.mode === mode && row.variant === variant);
    return {variant, mode, firstDecodeMs: distribution(rows.map(row => row.firstDecodeMs)),
      firstBinMs: distribution(rows.map(row => row.firstBinMs)),
      heapBytes: distribution(rows.map(row => row.metrics.JSHeapUsedSize)),
      resourceTransferBytes: distribution(rows.map(row => row.resources.reduce((sum, resource) => sum + resource.transferSize, 0))),
      inputToChangedMapMs: distribution(rows.flatMap(row => row.inputToChangedMapMs)),
      missingInputMapRecords: rows.flatMap(row => row.inputToChangedMapMs).filter(value => value === null).length,
      mapPrepareMs: distribution(rows.flatMap(row => row.mapTrace.frames.map(frame => frame[15]))),
      paneUiMs: distribution(rows.flatMap(row => (row.mapTrace.costs ?? []).map(cost => cost[4]))),
      longTaskDurationMs: distribution(rows.flatMap(row => row.longTasks.map(task => task.duration)))};
  }));
} finally {
  report.finished = new Date().toISOString();
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
  server?.close();
}
