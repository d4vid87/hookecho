// Delay display preparation after a decoded scan arrives: the prior same-product
// image must stay visible until its replacement is ready.
import puppeteer from 'puppeteer';
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import {fixtureRadar} from './fixtures/radar.mjs';

const output = process.env.HOOKECHO_CONTINUITY_OUTPUT ?? 'outputs/frame-continuity';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({headless: true,
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const report = {errors: []};
try {
  const page = await browser.newPage();
  await page.setViewport({width: 1200, height: 900});
  await page.setBypassServiceWorker(true);
  const requests = await fixtureRadar(page, {loopFrames: true, vector: true});
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true,
      seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly', live_loop_frames: 1}));
    let bridge;
    Object.defineProperty(globalThis, '__decodeVolume', {
      configurable: true, get: () => bridge,
      set: value => { bridge = async (bytes, op) => {
        const result = await value(bytes, op);
        if (op === 'bin') {
          if (globalThis.holdDisplay) {
            await new Promise(resolve => { globalThis.releaseDisplay = resolve; });
          }
          globalThis.displayJobs = (globalThis.displayJobs ?? 0) + 1;
        }
        return result;
      }; },
    });
  });
  const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
  await page.goto(base + '#goto=KDMX,-93.72,41.73,7', {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => globalThis.displayJobs > 0, {timeout: 90000});
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  await wait(1500);
  async function radarPixels(name) {
    const png = await page.screenshot({path: `${output}/${name}.png`,
      clip: {x: 180, y: 170, width: 600, height: 500}});
    return page.evaluate(async base64 => {
      const bytes = Uint8Array.from(atob(base64), char => char.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], {type: 'image/png'}));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      ctx.drawImage(bitmap, 0, 0);
      const {data} = ctx.getImageData(0, 0, canvas.width, canvas.height);
      let count = 0;
      for (let i = 0; i < data.length; i += 4) {
        if (data[i + 1] > 80 && data[i + 1] > data[i] * 1.5 && data[i + 1] > data[i + 2] * 1.5) count++;
      }
      return count;
    }, Buffer.from(png).toString('base64'));
  }
  report.before = await radarPixels('before');
  assert(report.before > 1000, 'fixture must have visible radar coverage');
  async function changeScan(name, action, retain) {
    await page.evaluate(() => { globalThis.holdDisplay = true; globalThis.releaseDisplay = null; });
    const jobs = await page.evaluate(() => globalThis.displayJobs);
    await action();
    await page.waitForFunction(() => globalThis.releaseDisplay, {timeout: 30000});
    await wait(500);
    const pending = await radarPixels(`${name}-pending`);
    await page.evaluate(() => { globalThis.holdDisplay = false; globalThis.releaseDisplay(); });
    await page.waitForFunction(jobs => globalThis.displayJobs > jobs, {timeout: 10000}, jobs);
    await wait(1000);
    const ready = await radarPixels(`${name}-ready`);
    report[name] = {pending, ready};
    assert(retain ? pending >= report.before * 0.98 : pending < report.before * 0.02,
      retain ? 'preparing the next scan must not blank the current radar image'
        : 'the old station must not masquerade as the new station');
    assert(ready >= report.before * 0.98, 'the replacement scan must render');
  }
  await page.mouse.click(700, 200);
  await changeScan('downloaded', () => page.keyboard.press('ArrowLeft'), true);
  // The previous step also prefetches its neighbors; exercise the decoded-cache handoff.
  const deadline = Date.now() + 10000;
  while (!requests.radarResponses.some(url => url.includes('_170300_')) && Date.now() < deadline) await wait(100);
  assert(requests.radarResponses.some(url => url.includes('_170300_')), 'the neighboring frame must be prefetched');
  await changeScan('cached', () => page.keyboard.press('ArrowLeft'), true);
  // Fixture bytes share coordinates, so only the radar identity changes in this check.
  await changeScan('station', () => page.goto(base + '#goto=KTLX,-93.72,41.73,7'), false);
  assert.equal(report.errors.length, 0);
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
