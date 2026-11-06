import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';
import {installUiTargets, clickTarget} from './fixtures/ui.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_WORKFLOW_OUTPUT ?? 'outputs/workflow-upgrades';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const errors = [];
const checks = [];
const page = await browser.newPage();
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function rendered() {
  const frame = await page.evaluate(() => {
    const frames = globalThis.__hookechoMapTrace?.frames;
    const latest = frames?.at(-1)?.[2] ?? 0;
    if (frames) frames.length = 0; // Keep the bounded trace buffer recording during long tests.
    return latest;
  });
  await page.waitForFunction(previous => globalThis.__hookechoMapTrace?.frames.at(-1)?.[2] >= previous + 2,
    {timeout: 30000}, frame);
}
async function command(text) {
  await page.focus('#hookecho');
  await page.keyboard.down('Control');
  await page.keyboard.press('KeyS');
  await page.keyboard.up('Control');
  // Wait for egui focus and results before sending more input on slower CI GPUs.
  await rendered();
  await page.mouse.click(200, 269); // Search field in the fixed 1280px desktop viewport.
  await rendered();
  await page.waitForFunction(() => document.activeElement?.tagName === 'INPUT', {timeout: 10000});
  await page.keyboard.type(text);
  await rendered();
  await page.keyboard.press('Enter');
  await rendered();
}
try {
  await page.setViewport({width: 1280, height: 900});
  await page.setBypassServiceWorker(true);
  await installUiTargets(page);
  await fixtureRadar(page, {loopFrames: true, forecast: true, vector: true, vectorResponse: async () => ({
    status: 200, contentType: 'application/json',
    body: JSON.stringify({tiles: [base + 'vector-fixture/{z}/{x}/{y}.pbf']}),
  })});
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (/panicked at|RuntimeError: unreachable/.test(message.text())) errors.push(message.text()); });
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    globalThis.__hookechoMapTraceEnabled = true;
    if (!localStorage.getItem('hookecho-settings')) localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
      radar_workspace: true, ui_scale: 1, youtube_key: 'PRIVATE-KEY-MUST-NOT-BE-SHARED',
      markers: [{id: 'test-dfw', name: 'Private home label', lon: -97.3, lat: 32.8, home: true}],
    }));
  });
  await page.goto(base, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await page.waitForFunction(() => location.hash.startsWith('#scene='), {timeout: 30000});
  await wait(2500);
  await page.screenshot({path: `${output}/desktop.png`});
  await clickTarget(page, 'Customize toolbar shortcuts');
  await wait(400);
  await page.screenshot({path: `${output}/shortcut-chooser.png`});
  await clickTarget(page, 'Pin toolbar shortcut: Drop shape (ZDR)');
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).compact_shortcuts?.length === 3);
  const pinned = await page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')).compact_shortcuts);
  assert.equal(pinned[0], 'mrms.composite-reflectivity');
  assert.equal(pinned[1], 'outlooks');
  assert.match(pinned[2], /^radar\.moment\./);
  await page.keyboard.press('Escape');
  await page.reload({waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  assert.deepEqual(await page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')).compact_shortcuts), pinned);
  checks.push('Toolbar product pin persists across reload alongside default MRMS and SPC');
  const scene = await page.evaluate(() => JSON.parse(atob(location.hash.slice(7).replaceAll('-', '+').replaceAll('_', '/'))));
  assert.equal(scene.time_mode, 'live');
  const serialized = JSON.stringify(scene);
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')).youtube_key), 'PRIVATE-KEY-MUST-NOT-BE-SHARED');
  assert(!serialized.includes('PRIVATE-KEY') && !serialized.includes('Private home label'), 'shared scenes must exclude credentials and saved places');
  checks.push('Live scene URL excludes credentials and saved places');

  const beforeScene = structuredClone(scene);
  scene.workspace.panes = [scene.workspace.panes[0], structuredClone(scene.workspace.panes[0])];
  scene.workspace.panes[1].site = 'KTLX';
  scene.workspace.panes[1].moment = 'DifferentialReflectivity';
  scene.workspace.active = 1;
  scene.workspace.link_cameras = true;
  scene.workspace.link_times = false;
  scene.workspace.chrome = {...scene.workspace.chrome, analyst_open: true};
  scene.products.storm_motion = [scene.products.storm_motion[0], scene.products.storm_motion[0]];
  scene.products.storm_motion[1] = [90, 50];
  scene.workspace.panes[1].storm_motion = [90, 50];
  scene.workspace.panes[1].dealias_override = false;
  scene.products.global_model = 'Ecmwf';
  scene.products.global_fcst_hour = 24;
  scene.filters.outlook_day = 3;
  scene.lock_source_frame = !beforeScene.lock_source_frame;
  scene.time_mode = 'exact';
  scene.times = ['2026-11-03T17:00:00Z', '2026-11-03T17:01:00Z'];
  await page.evaluate(token => { location.hash = 'scene=' + token; }, Buffer.from(JSON.stringify(scene)).toString('base64url'));
  await page.waitForFunction(() => {
    const state = JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session;
    return state?.workspace.panes.length === 2 && state.workspace.panes[1].site === 'KTLX';
  }, {timeout: 30000});
  await wait(2500);
  const restored = await page.evaluate(() => JSON.parse(atob(location.hash.slice(7).replaceAll('-', '+').replaceAll('_', '/'))));
  assert.deepEqual(restored.workspace.panes.map(p => p.site), ['KDMX', 'KTLX']);
  assert.equal(restored.workspace.panes[1].moment, 'DifferentialReflectivity');
  assert.equal(restored.time_mode, 'exact');
  assert.equal(restored.workspace.link_cameras, true);
  assert.equal(restored.workspace.link_times, false);
  checks.push('Same-tab scene restores independent radar sites and products with linked cameras');
  assert.equal(restored.workspace.panes[1].dealias_override, false);
  assert.deepEqual(restored.workspace.panes[1].storm_motion, [90, 50]);
  await command('Undo workspace action');
  await page.waitForFunction(expected => {
    if (!location.hash.startsWith('#scene=')) return false;
    const value = JSON.parse(atob(location.hash.slice(7).replaceAll('-', '+').replaceAll('_', '/')));
    return value.workspace.panes.length === 1
      && JSON.stringify(value.products) === JSON.stringify(expected.products)
      && JSON.stringify(value.filters) === JSON.stringify(expected.filters)
      && value.lock_source_frame === expected.lock_source_frame;
  }, {timeout: 30000}, beforeScene);
  checks.push('Undo restores shared-scene filters, model/hour, storm motion and source-time lock');
  await page.keyboard.press('Escape');
  await page.evaluate(token => { location.hash = 'scene=' + token; }, Buffer.from(JSON.stringify(scene)).toString('base64url'));
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.workspace.panes.length === 2,
    {timeout: 30000});
  await wait(1500);
  await page.screenshot({path: `${output}/two-pane-scene.png`});
  for (const [width, height] of [[390, 844], [320, 568]]) {
    await page.setViewport({width, height});
    await wait(1000);
    await page.screenshot({path: `${output}/phone-${width}.png`});
  }
  await page.setViewport({width: 1280, height: 900});
  await wait(500);
  await command('Location markers');
  await wait(1000);
  await page.screenshot({path: `${output}/saved-places.png`});
  await page.mouse.click(83, 515); // The saved DFW place's View radar here button.
  await page.waitForFunction(() => {
    const session = JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session;
    const pane = session?.workspace.panes[session.workspace.active];
    return pane && pane.site !== 'KTLX' && Math.abs(pane.lon + 97.3) < 0.001
      && Math.abs(pane.lat - 32.8) < 0.001 && session.panes[session.workspace.active].following;
  }, {timeout: 15000});
  checks.push('Saved-place radar action loads a nearby station, centers the place and returns to live');
  const compareStation = await page.evaluate(() => {
    const session = JSON.parse(localStorage.getItem('hookecho.recovery.v1')).session;
    return session.workspace.panes[session.workspace.active].site;
  });
  await command('Compare raw / corrected velocity');
  await page.waitForFunction(station => {
    const state = JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.workspace;
    return state?.panes.length === 2 && state.link_cameras && state.link_times
      && state.panes.every(pane => pane.site === station && pane.moment === 'Velocity')
      && state.panes[0].dealias_override === false && state.panes[1].dealias_override === true;
  }, {timeout: 30000}, compareStation);
  checks.push('Compare raw / corrected velocity retains the station and creates linked raw/corrected panes');
  await page.screenshot({path: `${output}/raw-corrected-velocity.png`});
  assert.deepEqual(errors, []);
} catch (error) {
  await page.screenshot({path: `${output}/failure.png`});
  await fs.writeFile(`${output}/failure-state.json`, JSON.stringify(await page.evaluate(() => ({
    hash: location.hash, recovery: JSON.parse(localStorage.getItem('hookecho.recovery.v1')),
  })), null, 2));
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify({checks, errors}, null, 2));
  await browser.close();
}
console.log(JSON.stringify({checks, errors}));
