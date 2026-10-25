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
    await page.mouse.click(810, 350);
    await new Promise(resolve => setTimeout(resolve, 50));
    await page.mouse.click(810, 350, {clickCount: 2});
    await page.waitForFunction(z =>
      Math.abs(globalThis.__hookechoMapTrace.frames.at(-1)[3] - z - 1) < 0.001,
      {timeout: 10000}, before);
    const after = await zoom();
    if (radarWorkspace) {
      await page.mouse.click(38, 45);
      await new Promise(resolve => setTimeout(resolve, 100));
      await page.mouse.click(38, 45, {clickCount: 2});
      await new Promise(resolve => setTimeout(resolve, 300));
      assert(Math.abs(await zoom() - after) < 0.001, 'toolbar double click must not zoom the map');
    }
    await page.mouse.click(930, 420, {button: 'right', clickCount: 2});
    await new Promise(resolve => setTimeout(resolve, 300));
    assert(Math.abs(await zoom() - after) < 0.001, 'right double click must not zoom');
    report.checks.push({radarWorkspace, doubleClickZoom: after - before});
    await page.removeScriptToEvaluateOnNewDocument(identifier);
  }
  assert.equal(report.errors.length, 0);
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
