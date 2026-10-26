import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

const output = process.env.HOOKECHO_SITE_OUTPUT ?? 'outputs/site-selector';
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
  await fixtureRadar(page, {loopFrames: true});
  page.on('pageerror', error => report.errors.push(error.message));
  page.on('console', message => {
    if (/panicked at|RuntimeError: unreachable/.test(message.text())) report.errors.push(message.text());
  });
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    // Preserve old favorites, with no recent_sites field: upgrades must start cleanly.
    localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'TDJT',
      radar_feed: 'ArchiveOnly', reduce_motion: true, presets: ['TDJT', 'KAMX'],
    }));
  });
  await page.goto((process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/') +
    '#goto=TDJT,-80.2730,26.6880,8.5,thr:16,bm:dark,proj:merc,pitch:0,brg:0', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await wait(2000);
  await page.evaluate(() => document.getElementById('dismiss-install')?.click());
  const settings = () => page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')));
  async function click(x, y) {await page.mouse.click(x, y); await wait(500);}
  async function open() {await click(605, 48);}
  async function search(text) {
    await click(250, 318);
    await page.keyboard.down('Control'); await page.keyboard.press('A'); await page.keyboard.up('Control');
    await page.keyboard.type(text, {delay: 50}); await wait(400);
  }
  await open();
  await page.screenshot({path: `${output}/nearby.png`});
  // Favorite button has its own hit target and must not select the station.
  await click(501, 510);
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).presets.includes('TFLL'));
  assert.equal((await settings()).recent_sites[0], 'TDJT');
  await click(62, 276); // Favorites
  await search('Miami');
  await page.screenshot({path: `${output}/favorite-search.png`});
  await click(240, 450); // KAMX is the only matching favorite
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).recent_sites[0] === 'KAMX');
  await open();
  await click(62, 344); // Recent
  await page.screenshot({path: `${output}/recent.png`});
  await search('TDJT');
  await click(240, 450);
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).recent_sites[0] === 'TDJT');
  assert.deepEqual((await settings()).recent_sites.slice(0, 2), ['TDJT', 'KAMX']);
  assert((await settings()).presets.includes('TFLL'));
  await open();
  for (const [network, x] of [['DWD', 327], ['OPERA', 384]]) {
    await click(x, 360);
    await page.screenshot({path: `${output}/${network}.png`});
  }
  await click(143, 360); // All
  const favoritesBeforeClose = (await settings()).presets;
  await click(501, 143); // Right-hand close, replacing fullscreen
  await page.screenshot({path: `${output}/closed.png`});
  await click(501, 450); // Former favorite target must no longer be active
  assert.deepEqual((await settings()).presets, favoritesBeforeClose);
  await open();
  for (const [width, height] of [[390, 844], [320, 568]]) {
    await page.setViewport({width, height}); await wait(800);
    await page.screenshot({path: `${output}/phone-${width}.png`});
  }
  assert.deepEqual(report.errors, []);
  report.checks.push('old favorites preserved', 'favoriting does not switch radar', 'search selects a real station',
    'recent stations persist in selection order', 'right close releases hit targets', 'desktop and narrow viewport captures');
} catch (error) {
  if (page) await page.screenshot({path: `${output}/failure.png`});
  report.failure = error.message;
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
