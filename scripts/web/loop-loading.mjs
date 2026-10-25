// Same-station quad startup: one head download/decode, shared by four panes.
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

const output = process.env.HOOKECHO_LOOP_OUTPUT ?? 'outputs/web-loop';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({headless: true, executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const report = {errors: []};
try {
  const page = await browser.newPage();
  await page.setViewport({width: 1440, height: 900});
  await page.setBypassServiceWorker(true);
  const requests = await fixtureRadar(page, {loopFrames: true, vector: true, networkState: {delayMs: 2000}});
  page.on('pageerror', error => report.errors.push(error.message));
  report.console = [];
  page.on('console', message => report.console.push(message.text()));
  const pane = {site: 'KTLX', moment: 'Reflectivity', tilt: 0, basemap: 'dark', lon: -93.72, lat: 41.73, zoom: 6.45, appearance: {}, fields_on: []};
  await page.evaluateOnNewDocument(pane => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    globalThis.__hookechoMapTraceEnabled = true;
    globalThis.__radarDecodes = 0;
    let bridge;
    Object.defineProperty(globalThis, '__decodeVolume', {
      configurable: true, get: () => bridge,
      set: value => { bridge = async (bytes, op) => {
        if (op === 'decode') globalThis.__radarDecodes++;
        return value(bytes, op);
      }; },
    });
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true, seeded_workspaces: true,
      default_site: 'KDMX', radar_feed: 'ArchiveOnly', live_loop_frames: 1,
      workspaces: [{name: 'Loading fixture', panes: Array.from({length: 4}, () => pane), active: 0,
        link_cameras: true, link_times: false, overlays_on: [], fields_on: [], adopt_site: false, chrome: {analyst_open: true}}]}));
  }, pane);
  await page.goto(process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  const opened = Date.now();
  await page.mouse.click(248, 38);
  await new Promise(resolve => setTimeout(resolve, 400));
  await page.mouse.click(200, 269);
  await page.keyboard.type('Loading fixture', {delay: 30});
  await new Promise(resolve => setTimeout(resolve, 500));
  await page.keyboard.press('Enter');
  await new Promise(resolve => setTimeout(resolve, 700));
  await page.mouse.click(702, 202);
  await page.waitForFunction(() => new Set(globalThis.__hookechoMapTrace?.frames.map(f => f[1])).size === 4, {timeout: 20000});
  await new Promise(resolve => setTimeout(resolve, 12000));
  report.elapsedMs = Date.now() - opened;
  report.requests = requests;
  report.decodes = await page.evaluate(() => globalThis.__radarDecodes);
  report.headDownloads = requests.radarRequests.filter(url => /KTLX[0-9]{8}_170500/.test(url));
  report.responses = requests.radarResponses.filter(url => /KTLX[0-9]{8}_170500/.test(url));
  report.saved = await page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')));
  await page.screenshot({path: `${output}/quad.png`});
  if (process.env.BASELINE !== '1') {
    assert.equal(report.headDownloads.length, 1, 'same-station panes must share one head download');
    assert(report.decodes >= 1 && report.decodes <= 2, 'one shared quad decode, plus the initial station if it finished before switching');
  }
  assert.equal(report.errors.length, 0);
  await page.close();
  console.log(JSON.stringify({phase: 'quad', decodes: report.decodes}));
  if (process.env.BASELINE !== '1') {
    const loopPage = await browser.newPage();
    await loopPage.setViewport({width: 1050, height: 900});
    await loopPage.setBypassServiceWorker(true);
    const loopRequests = await fixtureRadar(loopPage, {loopFrames: true, frameCount: 14, vector: true});
    loopPage.on('pageerror', error => report.errors.push(error.message));
    await loopPage.evaluateOnNewDocument(() => {
      Object.defineProperty(navigator, 'gpu', {value: undefined});
      if (!localStorage.getItem('loop-test-initialized')) {
        localStorage.clear();
        localStorage.setItem('loop-test-initialized', '1');
        localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true, seeded_workspaces: true,
          default_site: 'KDMX', radar_feed: 'ArchiveOnly', live_loop_frames: 14, web_radar_memory_mib: 768,
          // Memory-menu coordinates below target the HookEcho transport.
          radar_workspace: false}));
      }
    });
    await loopPage.goto(process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/', {waitUntil: 'domcontentloaded'});
    await loopPage.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
    const deadline = Date.now() + 45000;
    while (Date.now() < deadline && new Set(loopRequests.radarResponses.filter(url => /KDMX[0-9]{8}_17[0-9]{4}/.test(url))).size < 14) {
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    report.loopFramesDownloaded = new Set(loopRequests.radarResponses.filter(url => /KDMX[0-9]{8}_17[0-9]{4}/.test(url))).size;
    report.loopRequests = loopRequests;
    await loopPage.screenshot({path: `${output}/long-loop.png`});
    assert.equal(report.loopFramesDownloaded, 14, 'long loops must backfill every requested frame');
    await new Promise(resolve => setTimeout(resolve, 2500));
    await loopPage.screenshot({path: `${output}/long-loop.png`});
    await new Promise(resolve => setTimeout(resolve, 600));
    await loopPage.screenshot({path: `${output}/playback-controls.png`});
    await loopPage.mouse.move(750, 835);
    await new Promise(resolve => setTimeout(resolve, 150));
    await loopPage.mouse.click(750, 835);
    await new Promise(resolve => setTimeout(resolve, 500));
    await loopPage.screenshot({path: `${output}/frame-options.png`});
    await loopPage.mouse.move(570, 650);
    await new Promise(resolve => setTimeout(resolve, 150));
    await loopPage.mouse.click(570, 650);
    await new Promise(resolve => setTimeout(resolve, 1500));
    report.changedMemory = await loopPage.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')).web_radar_memory_mib);
    await loopPage.screenshot({path: `${output}/memory-changed.png`});
    assert.equal(report.changedMemory, 192, 'the visible memory selector must update saved settings');
    await loopPage.reload({waitUntil: 'domcontentloaded'});
    await loopPage.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
    report.restored = await loopPage.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')));
    assert.equal(report.restored.live_loop_frames, 14);
    assert.equal(report.restored.web_radar_memory_mib, 192);
    assert.equal(report.errors.length, 0);
  }
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify({headDownloads: report.headDownloads?.length, decodes: report.decodes, loopFramesDownloaded: report.loopFramesDownloaded, elapsedMs: report.elapsedMs, errors: report.errors}));
