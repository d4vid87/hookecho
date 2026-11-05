import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_RETRY_OUTPUT ?? 'outputs/feed-retry';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const page = await browser.newPage();
const report = {errors: [], warnings: [], checks: []};
try {
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  const requests = await fixtureRadar(page, {
    alertsResponse: async () => ({status: 429, contentType: 'application/json',
      headers: {'retry-after': '60', 'cache-control': 'no-store'}, body: '{"error":"rate limited"}'}),
    vector: true, vectorResponse: async () => ({status: 200, contentType: 'application/json',
      body: JSON.stringify({tiles: [`${new URL(base).origin}/vector-fixture/{z}/{x}/{y}.pbf`]}),
    }),
  });
  page.on('pageerror', error => report.errors.push(error.message));
  page.on('console', message => {
    const text = message.text();
    if (/panicked at|RuntimeError: unreachable/.test(text)) report.errors.push(text);
    if (/429|rate limit/i.test(text)) report.warnings.push(text);
  });
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    globalThis.__hookechoMapTraceEnabled = true;
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true,
      seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
      overlays_on: ['Alerts'], live_loop_frames: 1, reduce_motion: true}));
    globalThis.__feedResponses = [];
    // Observe the application's installed fetch wrapper. The test never issues a feed request.
    let fetch = globalThis.fetch.bind(globalThis);
    Object.defineProperty(globalThis, 'fetch', {configurable: true, get: () => fetch, set: wrapper => {
      fetch = async (input, init) => {
        const caller = new Error().stack;
        const response = await wrapper(input, init);
        const requested = input instanceof Request ? input.url : String(input);
        if (requested.includes('/proxy/api.weather.gov/')) {
          globalThis.__feedResponses.push({requested, url: response.url, status: response.status,
            retryAfter: response.headers.get('retry-after'), synthetic: Object.hasOwn(response, 'url'),
            requestObject: input instanceof Request, wasmCaller: /wasm-function|\.wasm/.test(caller)});
        }
        return response;
      };
    }});
    let decode;
    Object.defineProperty(globalThis, '__decodeVolume', {configurable: true, get: () => decode, set: worker => {
      decode = async (bytes, op) => {
        const result = await worker(bytes, op);
        if (op === 'bin') globalThis.__radarBinBytes = result.length;
        return result;
      };
    }});
  });
  await page.goto(base, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => globalThis.__feedResponses.some(response => response.synthetic), {timeout: 90000});
  await page.waitForFunction(() => globalThis.__radarBinBytes > 0 && globalThis.__hookechoMapTrace?.frames.length > 0, {timeout: 90000});
  // Let Rust consume the synthetic response and report its ordinary HTTP failure.
  await new Promise(resolve => setTimeout(resolve, 1500));
  report.responses = await page.evaluate(() => globalThis.__feedResponses);
  report.networkRequests = requests.alertsRequests;
  const actual = report.responses.filter(response => !response.synthetic);
  const synthetic = report.responses.filter(response => response.synthetic);
  assert(actual.length > 0 && synthetic.length > 0);
  assert.equal(requests.alertsRequests.length, actual.length, 'cooldown hits must not reach the network');
  assert(report.responses.every(response => response.requestObject && response.wasmCaller), 'all observed feed requests must come from the Rust client');
  for (const response of synthetic) {
    assert.equal(response.url, response.requested);
    assert.equal(response.status, 429);
    assert(Number(response.retryAfter) > 0 && Number(response.retryAfter) <= 60);
  }
  assert(report.warnings.some(text => /Weather alerts.*429/.test(text)), 'Rust must handle the retry as an ordinary HTTP error');
  assert(requests.radarResponses.length > 0);
  assert.deepEqual(report.errors, []);
  report.checks.push('Rust feed retry receives a URL-bearing synthetic 429 without another network request',
    'Rust reports the HTTP failure normally while radar decoding and map rendering continue');
  await page.screenshot({path: `${output}/throttled-feed.png`});
} catch (error) {
  report.failure = error.stack ?? String(error);
  await page.screenshot({path: `${output}/failure.png`}).catch(() => {});
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report, null, 2));
