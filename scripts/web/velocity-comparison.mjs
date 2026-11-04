// Real NOAA velocity gates/constants; fixture requests use synthetic listing times.
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_VELOCITY_OUTPUT ?? 'outputs/velocity-comparison';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({headless: true,
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const report = {checks: [], errors: [], limits: ['Historical NOAA sweep under synthetic listing times; software GPU; no live-feed or real-device latency claim.']};
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const page = await browser.newPage();
try {
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  await fixtureRadar(page, {fixturePath: new URL('../../crates/wxdata/tests/data/kfws-velocity-sweep.bin', import.meta.url),
    loopFrames: true, frameCount: 1, vector: true, vectorResponse: async () => ({status: 200,
      contentType: 'application/json', body: JSON.stringify({tiles: [base + 'vector-fixture/{z}/{x}/{y}.pbf']})})});
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    localStorage.clear();
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true,
      seeded_workspaces: true, default_site: 'KFWS', radar_feed: 'ArchiveOnly', live_loop_frames: 1,
      radar_workspace: true, ui_scale: 1}));
    globalThis.__velocityJobs = [];
    globalThis.__velocityReports = [];
    const create = URL.createObjectURL.bind(URL);
    URL.createObjectURL = blob => {
      blob.text().then(text => { try {
        const value = JSON.parse(text);
        if (value.schema === 'hookecho.diagnostics/v2') globalThis.__velocityReports.push(value);
      } catch {} });
      return create(blob);
    };
    let bridge;
    Object.defineProperty(globalThis, '__decodeVolume', {configurable: true, get: () => bridge,
      set: value => { bridge = async (bytes, op) => {
        const start = performance.now();
        // The worker transfers/detaches its input buffer, so inspect and copy before awaiting it.
        const input = op === 'bin' && bytes[bytes.length - 11] === 1 ? bytes.slice() : null;
        const result = await value(bytes, op);
        // bin payload ends in (Moment, f32 latitude, f32 longitude, bool dealias, bool precision).
        if (input) globalThis.__velocityJobs.push({
          corrected: input[input.length - 2] === 1, elapsedMs: performance.now() - start,
          input, result: result.slice(),
        });
        return result;
      }; },
    });
  });
  await page.goto(base + '#goto=KFWS,-97.3,32.6,7', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot') && location.hash.startsWith('#scene='), {timeout: 90000});
  await wait(1500);
  const command = async text => {
    await page.keyboard.down('Control'); await page.keyboard.press('KeyS'); await page.keyboard.up('Control');
    await wait(300); await page.keyboard.type(text); await wait(300); await page.keyboard.press('Enter');
  };
  const start = Date.now();
  await command('Compare raw / corrected velocity');
  await page.waitForFunction(() => {
    const session = JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session;
    return session?.workspace.panes.length === 2
      && session.workspace.panes.every(pane => pane.moment === 'Velocity' && pane.site === 'KFWS')
      && session.workspace.panes[0].dealias_override === false
      && session.workspace.panes[1].dealias_override === true
      && session.panes.every(pane => !pane.following)
      && globalThis.__velocityJobs.some(job => job.corrected)
      && globalThis.__velocityJobs.some(job => !job.corrected);
  }, {timeout: 90000});
  report.readyMs = Date.now() - start;
  await wait(2000);
  await command('Save diagnostics');
  await page.waitForFunction(() => globalThis.__velocityReports.length > 0, {timeout: 15000});
  const diagnostics = await page.evaluate(() => globalThis.__velocityReports.at(-1));
  const panes = diagnostics.reproduction.panes;
  assert.equal(panes.length, 2);
  assert(panes.every(pane => pane.moment === 'Velocity' && pane.displayed_station === 'KFWS' && !pane.load_failed));
  assert.equal(panes[0].volume_time, panes[1].volume_time);
  assert(panes[0].volume_time);
  assert(diagnostics.render_metrics.panes.every(pane => pane.binned_bytes > 0
    && pane.radar_upload_requested > 0 && pane.radar_upload_requested === pane.radar_upload_prepared));
  report.checks.push('Both velocity panes upload actual radar at the same source station and volume time');
  report.worker = await page.evaluate(async () => {
    const raw = globalThis.__velocityJobs.find(job => !job.corrected);
    const corrected = globalThis.__velocityJobs.find(job => job.corrected);
    const sameSource = raw.input.length === corrected.input.length
      && raw.input.every((value, i) => i === raw.input.length - 2 || value === corrected.input[i]);
    const glue = performance.getEntriesByType('resource').find(entry => /\/dist\/hookecho-[a-f0-9]+\.js/.test(entry.name)).name;
    const wasm = await import(glue);
    const jobs = [raw, corrected].map(job => {
      const inline = wasm.bin_radar_display(job.input.slice());
      return {corrected: job.corrected, elapsedMs: job.elapsedMs, outputBytes: job.result.length,
        matchesInline: inline.length === job.result.length && inline.every((value, i) => value === job.result[i])};
    });
    const changedOutputBytes = raw.result.reduce((count, value, i) => count + (value !== corrected.result[i]), 0);
    return {sameSource, changedOutputBytes, jobs};
  });
  assert(report.worker.sameSource, 'the worker payloads must differ only in correction choice');
  assert(report.worker.jobs.every(job => job.matchesInline), 'Nyquist metadata must survive page/worker serialization');
  assert(report.worker.changedOutputBytes > 0, 'correction must actually change velocity gates in this fixture');
  report.checks.push('Raw/corrected jobs share identical sweep bytes and match inline output with real Nyquist metadata');
  await page.keyboard.press('Escape');
  await wait(500);
  const screenshot = await page.screenshot({path: `${output}/raw-corrected.png`});
  report.radarPixels = await page.evaluate(async base64 => {
    const bitmap = await createImageBitmap(new Blob([Uint8Array.from(atob(base64), c => c.charCodeAt(0))], {type: 'image/png'}));
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext('2d'); ctx.drawImage(bitmap, 0, 0);
    // Exclude toolbar, playback and inspector. Count red/green velocity echoes in each map pane.
    return [[20, 120, 470, 620], [540, 120, 470, 620]].map(([x, y, width, height]) => {
      const {data} = ctx.getImageData(x, y, width, height); let count = 0;
      for (let i = 0; i < data.length; i += 4) if ((data[i] > 80 && data[i] > data[i + 1] * 1.5 && data[i] > data[i + 2] * 1.5)
        || (data[i + 1] > 80 && data[i + 1] > data[i] * 1.5 && data[i + 1] > data[i + 2] * 1.5)) count++;
      return count;
    });
  }, Buffer.from(screenshot).toString('base64'));
  assert(report.radarPixels.every(count => count > 1000), 'both maps must paint velocity coverage');
  report.checks.push('Both map panes paint visible velocity pixels; screenshot records processing labels');
  assert.deepEqual(report.errors, []);
} catch (error) {
  report.failureState = await page.evaluate(() => ({
    recovery: JSON.parse(localStorage.getItem('hookecho.recovery.v1')),
    jobs: globalThis.__velocityJobs?.map(job => ({corrected: job.corrected,
      elapsedMs: job.elapsedMs, inputBytes: job.input.length, outputBytes: job.result.length})),
  }));
  await page.screenshot({path: `${output}/failure.png`});
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
