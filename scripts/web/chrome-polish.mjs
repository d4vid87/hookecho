import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {fixtureRadar} from './fixtures/radar.mjs';
import {installUiTargets, clickTarget, targetPoint} from './fixtures/ui.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_CHROME_OUTPUT ?? 'outputs/chrome-polish';
await fs.mkdir(output, {recursive: true});
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const page = await browser.newPage();
const report = {checks: [], viewports: [], errors: []};
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function capture(file) {
  await page.evaluate(() => document.getElementById('dismiss-install')?.click());
  await page.screenshot({path: `${output}/${file}.png`});
}
const summary = 'Show warning details and controls';
const transport = ['Previous frame', 'Pause', 'Next frame', 'Choose map layout and pane count',
  'National mosaic (MRMS)', 'SPC convective outlook', 'Customize toolbar shortcuts',
  'Radar opacity', 'Map tools', 'Full playback controls'];
const top = ['Open workspace menu', 'Return to live radar', 'Weather source mode',
  'Search places and tools', 'Select radar station', 'Map projection', 'Open alerts'];

async function rectangles() {
  return page.evaluate(() => {
    const canvas = document.getElementById('hookecho');
    const box = canvas.getBoundingClientRect();
    return Object.fromEntries(Object.entries(globalThis.__hookechoUiTargets.controls)
      .map(([name, targets]) => [name, targets.map(({x, y, w, h, scale}) => ({
        x: box.left + x * scale * box.width / canvas.width,
        y: box.top + y * scale * box.height / canvas.height,
        w: w * scale * box.width / canvas.width,
        h: h * scale * box.height / canvas.height,
      }))]));
  });
}
function rect(all, name) {
  assert.equal(all[name]?.length, 1, `one visible ${name}`);
  return {...all[name][0], name};
}
function contained(r, width, height) {
  assert(r.x >= 0 && r.y >= 0 && r.x + r.w <= width + 1 && r.y + r.h <= height + 1,
    `${r.name} stays inside ${width}×${height}: ${JSON.stringify(r)}`);
}
function separate(a, b, gap = 0) {
  assert(a.x + a.w + gap <= b.x + 1 || b.x + b.w + gap <= a.x + 1
    || a.y + a.h + gap <= b.y + 1 || b.y + b.h + gap <= a.y + 1,
  `${a.name} overlaps ${b.name}`);
}
async function openAlertSettings() {
  await clickTarget(page, 'Open workspace menu');
  await clickTarget(page, 'Radar Settings');
  // Reopening Settings keeps its page during a session; reload returns to Settings home.
  const all = await rectangles();
  if (!all['Floating warning summary']) await clickTarget(page, 'Alerts');
}
async function summaryVisible(visible) {
  await page.waitForFunction(({name, visible}) =>
    Boolean(globalThis.__hookechoUiTargets?.controls[name]?.length) === visible,
  {timeout: 15000}, {name: summary, visible});
}
async function closeSettings() {
  await clickTarget(page, 'Close');
  // The recovery checkpoint is throttled to once per second; wait for the closed drawer
  // to reach that snapshot before reloading, not just the public URL (which omits drawers).
  await page.waitForFunction(() => {
    const saved = JSON.parse(localStorage.getItem('hookecho.recovery.v1') ?? 'null');
    return saved && !globalThis.__hookechoUiTargets?.controls.Close?.length
      && !saved.session.workspace.chrome?.drawer;
  }, {timeout: 15000});
}
async function pinShortcut(label, key) {
  await clickTarget(page, 'Customize toolbar shortcuts');
  const name = `Pin toolbar shortcut: ${label}`;
  // Adding a selected row changes the popup height and moves its checkbox rows.
  await page.waitForFunction(name => {
    const targets = globalThis.__hookechoUiTargets;
    const rows = targets?.controls[name];
    if (rows?.length !== 1) return false;
    const rect = JSON.stringify(rows[0]);
    const previous = globalThis.__hookechoStablePin;
    if (previous?.name === name && previous.pass === targets.pass) return previous.stable >= 2;
    const stable = previous?.name === name && previous.rect === rect ? previous.stable + 1 : 0;
    globalThis.__hookechoStablePin = {name, pass: targets.pass, rect, stable};
    return stable >= 2;
  }, {timeout: 30000}, name);
  await clickTarget(page, name);
  await page.waitForFunction(({label, key}) => {
    const saved = JSON.parse(localStorage.getItem('hookecho-settings'));
    return saved.compact_shortcuts?.includes(key)
      && globalThis.__hookechoUiTargets?.controls[label]?.length === 1;
  }, {timeout: 30000}, {label, key});
}

try {
  await page.setViewport({width: 1464, height: 1029});
  await page.setBypassServiceWorker(true);
  await installUiTargets(page);
  await fixtureRadar(page, {loopFrames: true, emptyOverlays: true, vector: true, vectorResponse: async () => ({
    status: 200, contentType: 'application/json',
    body: JSON.stringify({tiles: [`${new URL(base).origin}/vector-fixture/{z}/{x}/{y}.pbf`]}),
  })});
  page.on('pageerror', error => report.errors.push(error.message));
  page.on('console', message => {
    if (/panicked at|RuntimeError: unreachable/.test(message.text())) report.errors.push(message.text());
  });
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    // Older saved settings omit the new option; the floating summary must stay opt-in.
    if (!localStorage.getItem('hookecho-settings')) localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
      radar_workspace: true, reduce_motion: true, ui_scale: 1, overlays_on: ['Alerts'],
    }));
  });
  await page.goto(base, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await page.evaluate(() => document.getElementById('dismiss-install')?.click());
  await targetPoint(page, 'Open alerts');
  await summaryVisible(false);
  report.checks.push('Older saved settings start without a floating warning summary; alert access remains visible');

  for (const [width, height] of [[1464, 1029], [1280, 900], [1024, 900], [390, 844], [320, 720]]) {
    await page.setViewport({width, height});
    await wait(1200);
    const all = await rectangles();
    const targets = width >= 600
      ? [...top, ...transport.map(name => name === 'Pause' && !all.Pause ? 'Play' : name)]
      : ['Choose radar station', 'Search places, sites, products, and tools'];
    const controls = targets.map(name => rect(all, name));
    for (const r of controls) {
      contained(r, width, height);
      if (width >= 600) assert(Math.abs(r.h - 36) < 1, `${r.name} has a consistent 36px height: ${r.h}`);
      else assert(r.h >= 44, `${r.name} retains a touch target at least 44px high`);
    }
    for (let i = 0; i < controls.length; i++) {
      for (let j = i + 1; j < controls.length; j++) separate(controls[i], controls[j], 5);
    }
    if (width >= 600) {
      for (const [left, right] of [['Open workspace menu', 'Return to live radar'],
        ['Return to live radar', 'Weather source mode'], ['Weather source mode', 'Search places and tools'],
        ['Previous frame', all.Pause ? 'Pause' : 'Play'], [all.Pause ? 'Pause' : 'Play', 'Next frame'],
        ['National mosaic (MRMS)', 'SPC convective outlook']]) {
        const a = rect(all, left), b = rect(all, right);
        assert(Math.abs(a.y - b.y) < 1 && Math.abs(b.x - a.x - a.w - 6) < 1,
          `${left} and ${right} align with a 6px gap`);
      }
      const status = rect(all, 'Playback status — open full controls');
      contained(status, width, height);
      if (width >= 1100) {
        const product = rect(all, 'Choose radar product');
        contained(product, width, height);
        assert(Math.abs(product.h - 36) < 1, 'Product picker matches 36px controls');
        for (const control of controls) {
          separate(status, control, 5);
          separate(product, control, 5);
        }
        separate(status, product, 5);
      }
    }
    report.viewports.push({width, height, targets: all});
    await page.mouse.move(width / 2, height / 2);
    await capture(`${width}x${height}`);
  }
  report.checks.push('Desktop controls share heights and spacing, corner cards stay separate, and narrow/phone controls fit');

  await page.setViewport({width: 1464, height: 1029});
  await wait(1000);
  await clickTarget(page, 'Full playback controls');
  await clickTarget(page, 'Minimal playback controls');
  await targetPoint(page, 'Full playback controls');
  report.checks.push('Expanded playback returns to minimal controls');

  await openAlertSettings();
  await clickTarget(page, 'Floating warning summary');
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).show_warning_summary === true);
  await capture('warning-summary-setting');
  await closeSettings();
  await summaryVisible(true);
  await page.reload({waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await summaryVisible(true);
  await targetPoint(page, 'Open alerts');
  await capture('warning-summary-enabled');
  await openAlertSettings();
  await clickTarget(page, 'Floating warning summary');
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho-settings')).show_warning_summary === false);
  await closeSettings();
  await summaryVisible(false);
  const settings = await page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')));
  assert(settings.overlays_on.includes('Alerts'), 'Summary preference does not disable warning polygons');
  await targetPoint(page, 'Open alerts');
  report.checks.push('Summary can be enabled, survives reload, and can be disabled without disabling alert overlays or access');

  await page.setViewport({width: 1280, height: 900});
  await wait(1000);
  await pinShortcut('Rain intensity (reflectivity)', 'radar.moment.0.0');
  await pinShortcut('Drop shape (ZDR)', 'radar.moment.3.0');
  // Four shortcuts fit at 1280px; 1120px leaves just over 1100px inside the map frame,
  // exercising wrapping at the smallest split-card desktop.
  await page.setViewport({width: 1120, height: 900});
  await wait(1000);
  const pinned = await rectangles();
  const pinNames = [...transport.map(name => name === 'Pause' && !pinned.Pause ? 'Play' : name),
    'Rain intensity (reflectivity)', 'Drop shape (ZDR)'];
  const pinControls = pinNames.map(name => rect(pinned, name));
  assert(new Set(pinControls.map(r => Math.round(r.y))).size > 1, 'Four pinned shortcuts wrap inside the dock');
  for (let i = 0; i < pinControls.length; i++) {
    contained(pinControls[i], 1120, 900);
    separate(pinControls[i], rect(pinned, 'Playback status — open full controls'), 5);
    separate(pinControls[i], rect(pinned, 'Choose radar product'), 5);
    for (let j = i + 1; j < pinControls.length; j++) separate(pinControls[i], pinControls[j], 5);
  }
  await page.mouse.move(560, 400);
  await capture('four-shortcuts-wrap');
  report.checks.push('Four shortcuts can be pinned through the menu and wrap without overlapping the corner cards');

  await page.setViewport({width: 1280, height: 900});
  await page.evaluate(() => {
    const settings = JSON.parse(localStorage.getItem('hookecho-settings'));
    settings.theme = 'Light';
    localStorage.setItem('hookecho-settings', JSON.stringify(settings));
  });
  await page.reload({waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await page.evaluate(() => document.getElementById('dismiss-install')?.click());
  await targetPoint(page, 'Open alerts');
  await wait(1000);
  await capture('light-theme');
  report.checks.push('Light-theme chrome captured with the same four shortcuts for contrast review');
  assert.deepEqual(report.errors, []);
} catch (error) {
  report.failure = error.stack ?? String(error);
  report.visible = await rectangles().catch(() => ({}));
  await page.screenshot({path: `${output}/failure.png`}).catch(() => {});
  throw error;
} finally {
  await fs.writeFile(`${output}/report.json`, JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify({checks: report.checks, viewports: report.viewports.map(({width, height}) => ({width, height})), errors: report.errors}, null, 2));
