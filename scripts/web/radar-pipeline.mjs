// Real worker round-trips and fallback rendering, using the committed radar fixture.
// HOOKECHO_MRMS_FIXTURE optionally supplies an actual gzip MRMS GRIB2 object.
import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import {fixtureRadar} from './fixtures/radar.mjs';

const output = process.env.HOOKECHO_PIPELINE_OUTPUT ?? 'outputs/radar-pipeline';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({headless: true,
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const report = {errors: [], checks: {}, jobs: [], limits: ['Fixture/protocol checks, not live-feed or physical-device speed claims.']};
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
try {
  const page = await browser.newPage();
  await page.setViewport({width: 1200, height: 900});
  await page.setBypassServiceWorker(true);
  await fixtureRadar(page, {vector: true});
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    globalThis.__hookechoMapTraceEnabled = true;
    localStorage.clear();
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true,
      seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly', live_loop_frames: 1}));
    globalThis.__pipelineReports = [];
    const create = URL.createObjectURL.bind(URL);
    URL.createObjectURL = blob => {
      blob.text().then(text => {try {const value = JSON.parse(text); if (value.schema === 'hookecho.diagnostics/v2') globalThis.__pipelineReports.push(value);} catch {}});
      return create(blob);
    };
    let bridge;
    Object.defineProperty(globalThis, '__decodeVolume', {
      configurable: true, get: () => bridge,
      set: value => {
        globalThis.__pipelineBridge = value;
        bridge = async (bytes, op) => {
          if (op === 'bin') globalThis.__binInput ??= bytes.slice();
          if (op === 'bin' && globalThis.__pipelineNoBinWorker) throw Error('worker unavailable');
          const result = await value(bytes, op);
          if (op === 'bin') globalThis.__binResult ??= result.slice();
          return result;
        };
      },
    });
  });
  await page.goto(process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8766/', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => globalThis.__binResult && globalThis.__hookechoMapTrace?.frames.length > 0, {timeout: 90000});
  const bin = await page.evaluate(async () => {
    const glue = performance.getEntriesByType('resource').find(e => /\/dist\/hookecho-[a-f0-9]+\.js/.test(e.name)).name;
    const wasm = await import(glue);
    const direct = wasm.bin_radar_display(globalThis.__binInput.slice());
    const worker = globalThis.__binResult;
    return {equal: direct.length === worker.length && direct.every((v, i) => v === worker[i]), bytes: worker.length};
  });
  assert(bin.equal, 'worker binning must exactly match the inline fallback');
  report.checks.binRoundTrip = bin;

  // Exercise malformed jobs without retiring the worker, then prove the next job succeeds.
  assert(await page.evaluate(async () => {
    let rejected = 0;
    for (const op of ['bin', 'mrms']) {
      try { await globalThis.__pipelineBridge(new Uint8Array([255]), op); }
      catch { rejected++; }
    }
    const result = await globalThis.__pipelineBridge(globalThis.__binInput.slice(), 'bin');
    return rejected === 2 && result.length === globalThis.__binResult.length;
  }));
  report.checks.badJobRecovery = true;

  if (process.env.HOOKECHO_MRMS_FIXTURE) {
    // Postcard's string/vector lengths and usize are unsigned LEB128; Option<usize> adds a tag.
    const varint = number => {const bytes = []; do {const byte = number & 127; number = Math.floor(number / 128); bytes.push(byte | (number ? 128 : 0));} while (number); return Buffer.from(bytes);};
    const product = Buffer.from('CONUS/MergedReflectivityQCComposite_00.50');
    const gzip = await fs.readFile(process.env.HOOKECHO_MRMS_FIXTURE);
    const request = Buffer.concat([varint(product.length), product, varint(gzip.length), gzip, Buffer.from([1]), varint(512)]);
    report.mrmsFixture = 'supplied MRMS gzip GRIB2';
    report.jobs = await page.evaluate(async base64 => {
      const bytes = Uint8Array.from(atob(base64), c => c.charCodeAt(0));
      const glue = performance.getEntriesByType('resource').find(e => /\/dist\/hookecho-[a-f0-9]+\.js/.test(e.name)).name;
      const wasm = await import(glue);
      const results = [];
      let expected;
      for (const mode of ['worker', 'inline']) {
        let ticks = 0;
        const timer = setInterval(() => ticks++, 8);
        const start = performance.now();
        let result;
        try { result = mode === 'worker' ? await globalThis.__pipelineBridge(bytes.slice(), 'mrms') : wasm.decode_mrms(bytes.slice()); }
        finally { clearInterval(timer); }
        const elapsedMs = performance.now() - start;
        if (expected && (result.length !== expected.length || !result.every((v, i) => v === expected[i]))) throw Error('MRMS native/display worker output changed');
        expected = result;
        results.push({mode, elapsedMs, timerTicksDuringJob: ticks, outputBytes: result.length});
      }
      return results;
    }, request.toString('base64'));
    assert(report.jobs[0].timerTicksDuringJob > 0, 'the page must stay schedulable while MRMS decodes');
    report.checks.mrmsRoundTrip = true;
  } else {
    report.mrmsFixture = 'not supplied; set HOOKECHO_MRMS_FIXTURE to exercise MRMS round-trips';
  }
  await page.evaluateOnNewDocument(() => {globalThis.__pipelineNoBinWorker = true;});
  await page.reload({waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => globalThis.__binInput && !document.getElementById('boot'), {timeout: 90000});
  await wait(500);
  await page.mouse.click(600, 300);
  await page.keyboard.down('Control'); await page.keyboard.press('K'); await page.keyboard.up('Control');
  await wait(500); await page.keyboard.type('Save diagnostics', {delay: 30});
  await wait(500); await page.keyboard.press('Enter');
  await page.waitForFunction(() => globalThis.__pipelineReports.length > 0, {timeout: 15000});
  report.checks.inlineFallbackDraws = await page.evaluate(() => {
    const panes = globalThis.__pipelineReports.at(-1).render_metrics.panes;
    return panes.length > 0 && panes.every(p => p.radar_upload_requested > 0 && p.radar_upload_requested === p.radar_upload_prepared);
  });
  assert(report.checks.inlineFallbackDraws, 'unavailable workers must still produce uploaded radar');
  assert.deepEqual(report.errors, []);
  await wait(100);
  await page.screenshot({path: `${output}/radar.png`});
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
