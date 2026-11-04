import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_FORECAST_OUTPUT ?? 'outputs/forecast-recovery';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const errors = [];
const checks = [];
const response = temp => {
  const time = Math.floor(Date.now() / 3600000) * 3600 + 3600;
  return {status: 200, contentType: 'application/json', headers: {'Access-Control-Allow-Origin': '*'}, body: JSON.stringify({
    daily: {time: [time], temperature_2m_max: [temp], precipitation_probability_max: [20], weather_code: [0]},
    hourly: {time: [time], temperature_2m: [temp], precipitation_probability: [20], precipitation: [0]},
  })};
};
async function start(forecastResponse, markers = [], context = browser) {
  const page = await context.newPage();
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  await fixtureRadar(page, {loopFrames: true, forecastResponse, vector: true, vectorResponse: async () => ({
    status: 200, contentType: 'application/json', body: JSON.stringify({tiles: [base + 'vector-fixture/{z}/{x}/{y}.pbf']}),
  })});
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(markers => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    if (!localStorage.getItem('hookecho-settings')) localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
      radar_workspace: true, ui_scale: 1, markers,
    }));
  }, markers);
  await page.goto(base, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await wait(2000);
  return page;
}
async function command(page, text) {
  await page.keyboard.down('Control'); await page.keyboard.press('KeyK'); await page.keyboard.up('Control');
  await wait(400); await page.keyboard.type(text); await wait(400); await page.keyboard.press('Enter'); await wait(400);
}
async function until(predicate) {
  for (let i = 0; i < 100; i++) { if (predicate()) return; await wait(100); }
  assert(predicate(), 'expected forecast request did not arrive');
}
try {
  let releaseModels, releaseOther;
  const modelGate = new Promise(resolve => { releaseModels = resolve; });
  const otherGate = new Promise(resolve => { releaseOther = resolve; });
  let other = false, basicCount = 0, modelCount = 0;
  const page = await start(async url => {
    if (url.searchParams.has('models')) { modelCount++; await modelGate; return response(20); }
    basicCount++;
    const delayed = other;
    if (delayed) await otherGate;
    return response(delayed ? 99 : 20);
  });
  await command(page, 'Tool: Point forecast');
  await page.mouse.click(702, 202); // Close the search panel without leaving the forecast tool.
  await wait(300); await page.mouse.click(500, 450);
  await until(() => basicCount === 1 && modelCount === 3);
  await wait(1000);
  const clip = {x: 535, y: 195, width: 110, height: 65};
  const basicClip = {x: 535, y: 260, width: 110, height: 100};
  await page.screenshot({path: `${output}/basic-full.png`});
  const basic = await page.screenshot({clip: basicClip, path: `${output}/basic-before-comparisons.png`});
  releaseModels(); await wait(1500);
  await page.screenshot({path: `${output}/enriched-full.png`});
  const painted = await page.evaluate(async encoded => {
    const image = new Image(); image.src = 'data:image/png;base64,' + encoded; await image.decode();
    const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, image.width, image.height).data;
    let gold = 0;
    for (let i = 0; i < pixels.length; i += 4) if (pixels[i] > 245 && pixels[i + 1] > 195 && pixels[i + 1] < 220 && pixels[i + 2] < 125) gold++;
    return gold;
  }, Buffer.from(basic).toString('base64'));
  assert(painted > 200, 'basic forecast temperature must render while comparisons are pending');
  const enriched = await page.screenshot({clip});
  checks.push('Basic forecast renders while all three model comparisons are pending');
  other = true;
  await page.mouse.click(300, 450);
  await until(() => basicCount === 2);
  await wait(300);
  await page.mouse.click(500, 450); // Return to the cached first location while the other request waits.
  await wait(500);
  const cached = await page.screenshot({clip, path: `${output}/cached-location.png`});
  assert.deepEqual(cached, enriched);
  releaseOther(); await wait(1500);
  assert.deepEqual(await page.screenshot({clip, path: `${output}/after-late-response.png`}), cached,
    'a late result must not change the cached location forecast');
  assert.equal(basicCount, 2, 'returning to the cached point must not refetch it');
  assert.equal(modelCount, 3, 'an obsolete basic request must not launch comparisons');
  checks.push('Delayed other-location response cannot overwrite a cached forecast');
  await page.close();

  // An isolated profile ensures the saved-place card does not reuse the earlier detailed forecast.
  const context = await browser.createBrowserContext();
  let cardBasic = 0, cardModels = 0;
  const cards = await start(async url => {
    if (url.searchParams.has('models')) cardModels++; else cardBasic++;
    return response(70);
  }, [{id: 'dfw', name: 'DFW', lat: 32.8, lon: -97.3, home: true}], context);
  await command(cards, 'Location markers');
  await until(() => cardBasic === 1);
  await wait(1200);
  assert.equal(cardModels, 0, 'saved-place summaries must not request model comparisons');
  await cards.screenshot({path: `${output}/saved-place-basic.png`});
  checks.push('Saved-place cards fetch only the basic forecast');
  assert.deepEqual(errors, []);
  await context.close();
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify({checks, errors}, null, 2));
  await browser.close();
}
console.log(JSON.stringify({checks, errors}));
