// Local canvas interaction regression; requires Chromium, Puppeteer and Tesseract.
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {fixtureRadar} from './fixtures/radar.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8816/';
const output = process.env.HOOKECHO_TOOLS_OUTPUT ?? 'outputs/tool-controls';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  headless: true, args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
const page = await browser.newPage();
const report = {checks: [], errors: []};
const execute = promisify(execFile);
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function capture(name) {
  const path = `${output}/${name}.png`;
  const viewport = page.viewport();
  await page.mouse.move(viewport.width - 1, Math.floor(viewport.height / 2));
  await wait(250);
  await page.screenshot({path});
  const clip = name.startsWith('phone-')
    ? {x: 12, y: 156, width: viewport.width - 24, height: Math.min(350, viewport.height - 156)} : undefined;
  const ocrPath = clip ? `${output}/${name}-controls.png` : path;
  if (clip) await page.screenshot({path: ocrPath, clip});
  const readings = await Promise.all((clip ? ['11', '6'] : ['11']).map(mode =>
    execute(process.env.TESSERACT_BIN ?? 'tesseract', [ocrPath, 'stdout', '--psm', mode, 'tsv'])));
  const scale = viewport.deviceScaleFactor ?? 1;
  const words = readings.flatMap(({stdout}) => stdout.split('\n').slice(1)).map(line => line.split('\t')).filter(row => row.length >= 12 && row[11].trim())
    .map(row => ({text: row[11].replace(/^[^a-z0-9]+|[^a-z0-9]+$/gi, ''),
      x: +row[6] / scale + (clip?.x ?? 0), y: +row[7] / scale + (clip?.y ?? 0),
      width: +row[8] / scale, height: +row[9] / scale}));
  await fs.writeFile(`${output}/${name}.txt`, words.map(word => word.text).join(' '));
  return words;
}
async function click(words, label) {
  const word = words.find(word => word.text.toLowerCase() === label.toLowerCase());
  assert(word, `missing visible ${label}: ${words.map(word => word.text).join(' ')}`);
  await page.mouse.click(word.x + word.width / 2, word.y + word.height / 2);
  await wait(300);
}
async function command(text) {
  await page.keyboard.down('Control'); await page.keyboard.press('KeyS'); await page.keyboard.up('Control');
  await wait(250);
  await page.keyboard.type(text); await wait(250);
  if (page.viewport().width < 650) {
    const words = await capture(`search-${text.toLowerCase().replace(/[^a-z]+/g, '-')}`);
    const lastWord = text.split(' ').at(-1).toLowerCase();
    const result = words.filter(word => word.text.toLowerCase() === lastWord).at(-1);
    assert(result, `missing touch search result for ${text}`);
    await page.mouse.click(result.x + result.width / 2, result.y + result.height / 2);
  } else {
    await page.keyboard.press('Enter');
  }
  await wait(700);
}
async function exportDrawing(words) {
  const count = await page.evaluate(() => globalThis.__annotationExports.length);
  await click(words, 'Export');
  await page.waitForFunction(count => globalThis.__annotationExports.length > count, {}, count);
  return page.evaluate(() => globalThis.__annotationExports.at(-1));
}
try {
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  await fixtureRadar(page, {loopFrames: true, forecast: true, vector: true,
    vectorResponse: async () => ({status: 200, contentType: 'application/json',
      body: JSON.stringify({tiles: [base + 'vector-fixture/{z}/{x}/{y}.pbf']})})});
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true, seeded_workspaces: true,
      default_site: 'KDMX', radar_feed: 'ArchiveOnly', radar_workspace: true, ui_scale: 1}));
    globalThis.__annotationExports = [];
    const click = HTMLAnchorElement.prototype.click;
    HTMLAnchorElement.prototype.click = function () {
      if (this.download.endsWith('.geojson')) fetch(this.href).then(response => response.json())
        .then(value => globalThis.__annotationExports.push(value));
      else click.call(this);
    };
  });
  await page.goto(base, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await wait(1500);
  await page.$eval('#dismiss-install', button => button.click());
  await command('Tool: Point forecast');
  await page.keyboard.press('Escape'); await wait(400);
  const armed = await capture('escape-search-closed');
  assert(armed.some(word => word.text === 'Done'), 'first Escape must preserve the armed tool');
  await page.keyboard.press('Escape'); await wait(400);
  const disarmed = await capture('escape-tool-cancelled');
  assert(!disarmed.some(word => word.text === 'Done'), 'second Escape must cancel the tool');
  assert(disarmed.some(word => word.text.toLowerCase() === 'spc'), 'canceling a tool must keep radar controls visible');
  report.checks.push('Escape closes search, then cancels the tool without hiding radar controls');

  await page.setViewport({width: 390, height: 844}); await wait(500);
  await command('Tool: Draw');
  const drawing = await capture('phone-draw');
  for (const label of ['Done', 'Eraser', 'Undo', 'Redo', 'Clear', 'Export']) {
    assert(drawing.some(word => word.text === label), `${label} must be visible at phone width`);
  }
  for (const y of [480, 560]) {
    await page.mouse.move(70, y); await page.mouse.down();
    await page.mouse.move(240, y, {steps: 20}); await page.mouse.up();
  }
  await wait(500);
  await click(drawing, 'Undo');
  const one = await exportDrawing(drawing);
  assert.equal(one.features.length, 1, 'Undo removes the second stroke');
  await click(drawing, 'Redo');
  const two = await exportDrawing(drawing);
  assert.equal(two.features.length, 2, 'Redo restores the second stroke');
  await click(drawing, 'Eraser');
  await page.mouse.move(150, 450); await page.mouse.down();
  await page.mouse.move(150, 590, {steps: 20}); await page.mouse.up();
  const erased = await exportDrawing(drawing);
  assert.notDeepEqual(erased, two, 'Eraser changes the drawn geometry');
  await click(drawing, 'Done');
  const done = await capture('phone-draw-done');
  assert(!done.some(word => word.text === 'Eraser'), 'Done dismisses drawing controls');
  report.checks.push('390px drawing: controls visible; real strokes undo/redo, erase and GeoJSON export work');
  await command('Tool: Manual storm track');
  const track = await capture('phone-storm-track');
  for (const label of ['Done', 'Direction', 'Horizon']) {
    assert(track.some(word => word.text.replace(/[^a-z]/gi, '') === label), `${label} must be visible for a phone storm track`);
  }
  await page.mouse.move(280, 350); await page.mouse.wheel({deltaY: 200}); await wait(300);
  const trackScrolled = await capture('phone-storm-track-scrolled');
  for (const label of ['Done', 'Speed', 'Cone', 'Clear']) {
    assert(trackScrolled.some(word => word.text === label), `${label} must remain reachable for a phone storm track`);
  }
  report.checks.push('390px manual storm track exposes motion/cone controls through a scrollable body with fixed Done');
  await page.setViewport({width: 320, height: 568}); await wait(500);
  await page.mouse.move(280, 285); await page.mouse.wheel({deltaY: -600}); await wait(300);
  const smallTrack = await capture('phone-320-storm-track');
  assert(smallTrack.some(word => word.text === 'Done'), 'Done stays visible on a small phone');
  await page.mouse.move(280, 285); await page.mouse.wheel({deltaY: 65}); await wait(300);
  const smallTrackDirection = await capture('phone-320-storm-track-direction');
  assert(smallTrackDirection.some(word => word.text === 'Direction'), 'small-phone direction label must stay fully reachable');
  await page.mouse.move(280, 285); await page.mouse.wheel({deltaY: 200}); await wait(300);
  const smallTrackScrolled = await capture('phone-320-storm-track-scrolled');
  for (const label of ['Speed', 'Cone', 'Clear']) {
    assert(smallTrackScrolled.some(word => word.text === label), `small-phone ${label} stays fully reachable by scrolling`);
  }
  await click(smallTrack, 'Done');
  await command('Tool: Draw');
  const smallDrawing = await capture('phone-320-draw');
  assert(smallDrawing.some(word => word.text === 'Done'), 'drawing can be dismissed on a small phone');
  await page.mouse.move(280, 285); await page.mouse.wheel({deltaY: 200}); await wait(300);
  const smallDrawingScrolled = await capture('phone-320-draw-scrolled');
  assert(smallDrawingScrolled.some(word => word.text === 'Export'), 'small-phone drawing export stays reachable');
  report.checks.push('320px phone keeps Done visible and scrolls to drawing/track options');
  assert.deepEqual(report.errors, []);
} catch (error) {
  report.errors.push(error.stack ?? String(error));
  await page.screenshot({path: `${output}/failure.png`}).catch(() => {});
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(report.checks.join('\n'));
