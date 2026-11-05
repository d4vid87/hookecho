// Exercise composite navigation and the real browser decode bridge with a tiny observed grid.
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_WORLD_OUTPUT ?? 'outputs/worldwide';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true, args: [
  '--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader',
]});
const errors = [];
const page = await browser.newPage();
try {
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (/panicked at|RuntimeError: unreachable/.test(message.text())) errors.push(message.text());
  });
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'TDJT',
      radar_workspace: true, reduce_motion: true,
    }));
    globalThis.__worldWorkerOps = [];
    let bridge;
    Object.defineProperty(globalThis, '__decodeVolume', {
      configurable: true,
      get: () => bridge,
      set: value => { bridge = async (bytes, op) => {
        const result = await value(bytes, op);
        globalThis.__worldWorkerOps.push(op);
        return result;
      }; },
    });
  });
  const observed = new Date(Math.floor(Date.now() / 300000) * 300000 - 300000);
  const stamp = observed.toISOString().replace(/[-:TZ.]/g, '').slice(0, 14);
  const tile = await fs.readFile(new URL('../../web/icon-192.png', import.meta.url));
  await page.setRequestInterception(true);
  page.on('request', async request => {
    const url = new URL(request.url());
    if (url.href.includes('O-A0059-001.xml')) {
      await request.respond({status: 200, contentType: 'application/xml', body:
        `<data><dataid>O-A0059-001</dataid><Reflectivity>dBZ</Reflectivity><contentDescription>TWD67</contentDescription>` +
        `<DateTime>${observed.toISOString()}</DateTime><GridDimensionX>2</GridDimensionX><GridDimensionY>2</GridDimensionY>` +
        `<StartPointLongitude>121</StartPointLongitude><StartPointLatitude>24</StartPointLatitude><GridResolution>0.1</GridResolution>` +
        `<content>35,-99,-999,45</content></data>`});
    } else if (url.href.includes('targetTimes_N1.json')) {
      await request.respond({status: 200, contentType: 'application/json', body: JSON.stringify([
        {basetime: stamp, validtime: stamp, elements: ['hrpns']},
      ])});
    } else if (url.href.includes('jma.go.jp/bosai/jmatile/') && url.pathname.endsWith('.png')) {
      await request.respond({status: 200, contentType: 'image/png', body: tile});
    } else if (url.href.includes('tiles.openfreemap.org')) {
      await request.respond({status: 200, contentType: 'application/json', body: JSON.stringify({tiles: [base + 'vector-fixture/{z}/{x}/{y}.pbf']})});
    } else if (url.pathname.startsWith('/vector-fixture/')) {
      await request.respond({status: 200, contentType: 'application/x-protobuf', body: Buffer.alloc(0)});
    } else if (url.pathname.startsWith('/proxy/') || url.origin !== new URL(base).origin) {
      await request.respond({status: 503, body: 'Fixture excludes unrelated feeds'});
    } else await request.continue();
  });
  await page.goto(base + '#goto=TDJT,121,24,6,bm:taiwan-radar', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await page.waitForFunction(() => globalThis.__worldWorkerOps.includes('cwa'), {timeout: 30000});
  const scene = () => page.evaluate(() => JSON.parse(atob(location.hash.slice(7).replaceAll('-', '+').replaceAll('_', '/'))));
  await page.waitForFunction(() => location.hash.startsWith('#scene='));
  let current = await scene();
  assert.equal(current.workspace.panes[0].basemap, 'taiwan-radar');
  assert.equal(current.workspace.panes[0].show_radar, false);
  await page.screenshot({path: `${output}/taiwan.png`});
  await page.evaluate(() => { location.hash = 'goto=TDJT,139.7,35.7,6,bm:jma-radar'; });
  await page.waitForFunction(() => location.hash.startsWith('#scene='));
  await new Promise(resolve => setTimeout(resolve, 2200));
  current = await scene();
  assert.equal(current.workspace.panes[0].basemap, 'jma-radar');
  assert.equal(current.workspace.panes[0].show_radar, false);
  await page.screenshot({path: `${output}/japan.png`});
  assert.deepEqual(errors, []);
  console.log('Worldwide browser checks passed: CWA worker decode, composite controls, explicit source navigation.');
} finally {
  await browser.close();
}
