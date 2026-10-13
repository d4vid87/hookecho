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
  assert.equal(report.errors.length, 0);
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
