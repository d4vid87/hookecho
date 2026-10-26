import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

// Browser viewport checks; these do not replace physical-phone usability testing.
const output = process.env.HOOKECHO_USABILITY_OUTPUT ?? 'outputs/usability';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const report = {checks: [], errors: []};
try {
  const page = await browser.newPage();
  await page.setBypassServiceWorker(true);
  await fixtureRadar(page);
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    globalThis.__hookechoMapTraceEnabled = true;
    localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
    }));
  });
  for (const [width, height] of [[390, 844], [320, 568]]) {
    await page.setViewport({width, height, deviceScaleFactor: 1});
    await page.goto(process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/', {waitUntil: 'domcontentloaded'});
    await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
    await page.evaluate(() => {
      const event = new Event('beforeinstallprompt', {cancelable: true});
      event.prompt = () => Promise.resolve();
      event.userChoice = Promise.resolve({outcome: 'dismissed'});
      dispatchEvent(event);
    });
    await page.waitForSelector('#install-prompt', {visible: true});
    const geometry = await page.evaluate(() => {
      const box = document.getElementById('install-prompt').getBoundingClientRect();
      return {left: box.left, right: box.right, top: box.top, bottom: box.bottom,
        heights: [...document.querySelectorAll('#install-prompt button')].map(button => button.getBoundingClientRect().height)};
    });
    assert(geometry.left >= 0 && geometry.right <= width, 'install prompt must fit horizontally');
    assert(geometry.top >= 0, 'install prompt must remain on screen');
    assert(geometry.bottom <= height - 264, 'install prompt must reserve the playback/navigation area');
    assert(geometry.heights.every(value => value >= 44), 'install actions must have 44px touch targets');
    await page.screenshot({path: `${output}/phone-${width}-install.png`});
    await page.click('#dismiss-install');
    await page.waitForSelector('#install-prompt', {hidden: true});
    await page.screenshot({path: `${output}/phone-${width}-dismissed.png`});
    report.checks.push({width, height, geometry, dismissed: true});
  }
  await page.setViewport({width: 1280, height: 800, deviceScaleFactor: 1});
  for (const radarWorkspace of [true, false]) {
    const {identifier} = await page.evaluateOnNewDocument(value => {
      const settings = JSON.parse(localStorage.getItem('hookecho-settings'));
      settings.radar_workspace = value;
      settings.reduce_motion = !value;
      localStorage.setItem('hookecho-settings', JSON.stringify(settings));
    }, radarWorkspace);
    // Each layout starts regionally; the previous station test persists a maximum-zoom URL.
    await page.goto((process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/') +
      '#goto=KDMX,-93.7228,41.7311,6.45,thr:16,bm:dark,proj:merc,pitch:0,brg:0',
      {waitUntil: 'domcontentloaded'});
    await page.reload({waitUntil: 'domcontentloaded'});
    await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
    await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.panes[0]?.time,
      {timeout: 90000});
    // Let the fixture decode/upload settle before measuring camera animation.
    await new Promise(resolve => setTimeout(resolve, 3000));
    await page.waitForFunction(() => globalThis.__hookechoMapTrace?.frames.length > 0);
    const zoom = () => page.evaluate(() => globalThis.__hookechoMapTrace.frames.at(-1)[3]);
    const before = await zoom();
    await page.mouse.click(810, 350);
    await new Promise(resolve => setTimeout(resolve, 100));
    assert(Math.abs(await zoom() - before) < 0.001, 'single click must not zoom');
    // Keep assertions outside the double-click timing window on slow CI renderers.
    await new Promise(resolve => setTimeout(resolve, 750));
    await page.mouse.click(810, 350, {count: 2});
    await page.waitForFunction(z =>
      Math.abs(globalThis.__hookechoMapTrace.frames.at(-1)[3] - z - 1) < 0.001,
      {timeout: 10000}, before);
    const after = await zoom();
    if (radarWorkspace) {
      await page.mouse.click(38, 45, {count: 2});
      await new Promise(resolve => setTimeout(resolve, 300));
      assert(Math.abs(await zoom() - after) < 0.001, 'toolbar double click must not zoom the map');
    }
    await page.mouse.click(930, 420, {button: 'right', count: 2});
    await new Promise(resolve => setTimeout(resolve, 300));
    assert(Math.abs(await zoom() - after) < 0.001, 'right double click must not zoom');
    report.checks.push({radarWorkspace, doubleClickZoom: after - before});
    // Test both station dots and the selected station pill, including a first-click recenter.
    for (const target of [
      {id: 'KTLX', lon: -97.2778, lat: 35.3331, pill: false},
      {id: 'KDMX', lon: -93.7228, lat: 41.7311, pill: true},
    ]) {
      await page.goto((process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/') +
        '#goto=KDMX,' + (target.lon - 7) + ',' + target.lat + ',5.1,thr:16,bm:dark,proj:merc,pitch:0,brg:0',
        {waitUntil: 'domcontentloaded'});
      await page.reload({waitUntil: 'domcontentloaded'});
      await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
      await new Promise(resolve => setTimeout(resolve, 2000));
      const point = await page.evaluate(({lon, lat, pill}) => {
        const f = globalThis.__hookechoMapTrace.frames.at(-1);
        const world = [(lon + 180) / 360, (1 - Math.log(Math.tan(Math.PI / 4 + lat * Math.PI / 360)) / Math.PI) / 2];
        const scale = 256 * 2 ** f[3];
        return {x: (1280 - f[6]) / 2 + f[6] / 2 + (world[0] - f[4]) * scale + (pill ? 45 : 0),
          y: (800 - f[7]) / 2 + f[7] / 2 + (world[1] - f[5]) * scale, world};
      }, target);
      await page.mouse.click(point.x, point.y, {count: 2, delay: 60});
      await page.waitForFunction(() => Math.abs(globalThis.__hookechoMapTrace.frames.at(-1)[3] - 20) < 0.001,
        {timeout: 15000});
      const f = await page.evaluate(() => globalThis.__hookechoMapTrace.frames.at(-1));
      assert(Math.abs(f[4] - point.world[0]) < 1e-6 && Math.abs(f[5] - point.world[1]) < 1e-6,
        'maximum zoom must be centered on the chosen station');
      await page.waitForFunction(id => JSON.parse(localStorage.getItem('hookecho-settings')).recent_sites[0] === id,
        {}, target.id);
      await page.screenshot({path: output + '/site-' + target.id + '-' + radarWorkspace + '.png'});
      report.checks.push({radarWorkspace, station: target.id, pill: target.pill, zoom: f[3]});
    }
    await page.removeScriptToEvaluateOnNewDocument(identifier);
  }
  assert.equal(report.errors.length, 0);
} catch (error) {
  const page = (await browser.pages()).at(-1);
  report.failureTrace = await page.evaluate(() => globalThis.__hookechoMapTrace);
  await page.screenshot({path: `${output}/failure.png`});
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
