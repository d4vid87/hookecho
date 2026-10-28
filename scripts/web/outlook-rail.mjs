import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

// Fixed canvas coordinates deliberately check the compact toolbar and category hit targets.
const output = process.env.HOOKECHO_OUTLOOK_OUTPUT ?? 'outputs/outlook-rail';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const report = {errors: [], checks: []};
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
let page;
try {
  page = await browser.newPage();
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  const requests = await fixtureRadar(page, {outlooks: true});
  page.on('pageerror', error => report.errors.push(error.message));
  page.on('console', message => {
    if (/panicked at|RuntimeError: unreachable/.test(message.text())) report.errors.push(message.text());
  });
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
    }));
  });
  await page.goto(process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await wait(2000);
  await page.mouse.click(652, 856); // MRMS immediately left of SPC
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).recent_fields?.includes('mrms.composite-reflectivity'));
  await page.screenshot({path: `${output}/mrms-on.png`});
  await page.mouse.click(652, 856); // Restore the overlay before testing SPC
  await wait(500);
  report.checks.push('minimal MRMS shortcut enables existing reflectivity layer');
  // SPC must take over from Alerts, not inherit its panel section or visibility flag.
  await page.mouse.click(1225, 40);
  await wait(800);
  await page.screenshot({path: `${output}/alerts-before-spc.png`});
  await page.mouse.click(725, 856); // SPC, immediately right of MRMS in minimal controls
  await wait(800);
  assert.equal(requests.outlookRequests.length, 0, 'opening outlooks must not enable an overlay');
  await page.screenshot({path: `${output}/severe-off.png`});
  await page.mouse.click(245, 400); // Day 2
  for (let i = 0; i < 40 && requests.outlookRequests.length === 0; i++) await wait(250);
  assert(requests.outlookRequests.some(url => url.includes('day2otlk_cat')), 'minimal SPC shortcut must open working day controls');
  const enabledRequests = requests.outlookRequests.length;
  for (const [name, y] of [['fire', 310], ['rain', 375], ['winter', 443], ['climate', 507], ['severe', 246]]) {
    await page.mouse.click(77, y);
    await wait(500);
    await page.screenshot({path: `${output}/${name}.png`});
    assert.equal(requests.outlookRequests.length, enabledRequests, 'category browsing must preserve the enabled outlook');
  }
  await page.mouse.click(309, 400); // Day 3 after returning to Severe
  for (let i = 0; i < 40 && requests.outlookRequests.length === enabledRequests; i++) await wait(250);
  assert(requests.outlookRequests.some(url => url.includes('day3otlk_cat')), 'returning to Severe must retain functional controls');
  report.checks.push('minimal SPC replaces Alerts with Signal rail', 'opening is passive', 'category switching preserves outlook', 'day changes fetch selected NOAA product');
  for (const [width, height] of [[390, 844], [320, 568]]) {
    await page.setViewport({width, height});
    await wait(700);
    await page.screenshot({path: `${output}/phone-${width}.png`});
  }
  assert.deepEqual(report.errors, []);
  report.requests = requests.outlookRequests;
} catch (error) {
  if (page) await page.screenshot({path: `${output}/failure.png`});
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
