import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {fixtureRadar} from './fixtures/radar.mjs';
import {installUiTargets, clickTarget} from './fixtures/ui.mjs';

const base = process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/';
const output = process.env.HOOKECHO_BUNDLE_OUTPUT ?? 'outputs/settings-bundle';
await fs.mkdir(output, {recursive: true});
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'hookecho-settings-bundle-'));
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const report = {checks: [], errors: []};
const page = await browser.newPage();
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const legacyGis = 'gis:legacy.geojson';
const largeGis = 'gis:large.geojson';
const point = JSON.stringify({type: 'Point', coordinates: [-93.6, 41.6]});
const largeSource = point + ' '.repeat(6 * 1024 * 1024);
const initial = {setup_done: true, seeded_workspaces: true, default_site: 'KDMX',
  radar_feed: 'ArchiveOnly', radar_workspace: false, reduce_motion: true, ui_scale: 1,
  overlays_on: ['Alerts', 'Cells'], web_files: {[legacyGis]: point},
  placefiles: [{url: legacyGis, enabled: true, opacity: 1}]};
const palette = '; browser bundle regression\nProduct: BR\nUnits: dBZ\nStep: 5\nColor: -30 1 2 3\nColor: 80 240 250 255\n';
const settings = () => page.evaluate(() => JSON.parse(localStorage.getItem('hookecho-settings')));
async function openBackup() {
  await clickTarget(page, 'Radar toolbar menu');
  await clickTarget(page, 'Open Quick Launch');
  await clickTarget(page, 'Tools');
  await clickTarget(page, 'Share this view');
  await clickTarget(page, 'Back to preferences');
  await clickTarget(page, 'Backup');
}
async function importFile(file) {
  const [chooser] = await Promise.all([
    page.waitForFileChooser(), clickTarget(page, 'Restore settings backup'),
  ]);
  await chooser.accept([file]);
}
async function assertImported() {
  await page.waitForFunction(palette => {
    const value = JSON.parse(localStorage.getItem('hookecho-settings'));
    return value?.web_files?.[value.palettes?.REF] === palette
      && value.overlays_on?.includes('Metar') && !value.overlays_on.includes('Alerts');
  }, {timeout: 15000}, palette);
  // Check a later autosave too: the original defect replaced imported flags with stale live flags.
  await wait(1500);
  const value = await settings();
  assert.deepEqual([...value.overlays_on].sort(), ['Metar', 'Tropical']);
  assert.equal(value.web_files[value.palettes.REF], palette);
  assert.equal(value.plugins[0].enabled, false);
  assert.equal(value.plugins[0].command, 'must-not-run-imported-plugin');
  assert.match(value.web_files[largeGis], /^indexeddb:sha256:[a-f0-9]{64}$/);
  assert.equal(value.placefiles.find(layer => layer.url === largeGis).enabled, true);
}
try {
  await page.setViewport({width: 1280, height: 1100});
  await page.setBypassServiceWorker(true);
  await installUiTargets(page);
  await fixtureRadar(page, {vector: true, vectorResponse: async () => ({
    status: 200, contentType: 'application/json',
    body: JSON.stringify({tiles: [`${new URL(base).origin}/vector-fixture/{z}/{x}/{y}.pbf`]}),
  })});
  page.on('pageerror', error => report.errors.push(error.message));
  page.on('console', message => {
    if (/panicked at|RuntimeError: unreachable/.test(message.text())) report.errors.push(message.text());
  });
  await page.evaluateOnNewDocument(initial => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    if (!localStorage.getItem('hookecho-settings')) localStorage.setItem('hookecho-settings', JSON.stringify(initial));
    globalThis.__settingsDownloads = [];
    const click = HTMLAnchorElement.prototype.click;
    HTMLAnchorElement.prototype.click = function () {
      if (this.download === 'hookecho-settings.json') {
        fetch(this.href).then(response => response.text()).then(bytes => globalThis.__settingsDownloads.push(bytes));
      } else click.call(this);
    };
  }, initial);
  await page.goto(base, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await page.evaluate(() => document.getElementById('dismiss-install')?.click());
  assert.match((await settings()).web_files[legacyGis], /^indexeddb:sha256:[a-f0-9]{64}$/);
  report.checks.push('Startup migrates legacy inline GIS to browser file storage');
  await openBackup();
  const importedPath = path.join(temporary, 'desktop-settings.json');
  await fs.writeFile(importedPath, JSON.stringify({
    settings: {...initial, overlays_on: ['Metar'], palettes: {REF: '/another-machine/custom.pal'},
      web_files: {...initial.web_files, [largeGis]: largeSource},
      placefiles: [...initial.placefiles, {url: largeGis, enabled: true, opacity: 1}],
      plugins: [{name: 'Imported test plugin', command: 'must-not-run-imported-plugin', enabled: true}]},
    palette_files: {REF: palette},
  }));
  await importFile(importedPath);
  await assertImported();
  report.checks.push('Real file picker imports desktop palette bytes, replaces live overlays, and disables plugins through autosave');
  report.checks.push('A 6 MiB GIS import uses an IndexedDB reference and leaves preferences writable');
  await clickTarget(page, 'Save settings backup');
  await page.waitForFunction(() => globalThis.__settingsDownloads.length === 1);
  const exported = await page.evaluate(() => globalThis.__settingsDownloads[0]);
  const bundle = JSON.parse(exported);
  assert.equal(bundle.palette_files.REF, palette);
  assert.equal(bundle.settings.plugins[0].enabled, false);
  assert.equal(bundle.settings.web_files[legacyGis], point);
  assert.ok(bundle.settings.web_files[largeGis] === largeSource, 'backup preserves original GIS bytes');
  assert.deepEqual([...bundle.settings.overlays_on].sort(), ['Metar', 'Tropical']);
  const exportedPath = path.join(temporary, 'exported-settings.json');
  await fs.writeFile(exportedPath, exported);
  report.checks.push('Browser export contains the original custom palette bytes and imported overlay choices');
  await page.screenshot({path: path.join(output, 'imported.png')});

  await page.reload({waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await assertImported();
  report.checks.push('Imported palette, overlay choices, and disabled plugin survive reload');
  await openBackup();
  await clickTarget(page, 'Save settings backup');
  await page.waitForFunction(() => globalThis.__settingsDownloads.length === 1);
  assert.ok(await page.evaluate(({largeGis, size}) => {
    const bundle = JSON.parse(globalThis.__settingsDownloads[0]);
    return bundle.settings.web_files[largeGis]?.length === size;
  }, {largeGis, size: largeSource.length}), 'reload resolves the IndexedDB source for portable export');
  report.checks.push('Original GIS bytes are available for portable export after reload');

  const resetPath = path.join(temporary, 'reset-settings.json');
  await fs.writeFile(resetPath, JSON.stringify({settings: initial, palette_files: {}}));
  await importFile(resetPath);
  await page.waitForFunction(() => {
    const value = JSON.parse(localStorage.getItem('hookecho-settings'));
    return !value.palettes?.REF && value.overlays_on?.includes('Alerts') && !value.overlays_on.includes('Metar');
  }, {timeout: 15000});
  await importFile(exportedPath);
  await assertImported();
  report.checks.push('Exported bytes restore the same settings through a second real file-picker import');

  const rejectedGis = 'gis:rejected.geojson';
  const rejectedPath = path.join(temporary, 'rejected-settings.json');
  await fs.writeFile(rejectedPath, JSON.stringify({settings: {
    ...initial, web_files: {[rejectedGis]: point},
    placefiles: [{url: rejectedGis, enabled: true, opacity: 1}],
  }, palette_files: {}}));
  await page.evaluate(() => {
    const put = IDBObjectStore.prototype.put;
    globalThis.__rejectGisWrite = true;
    IDBObjectStore.prototype.put = function (...args) {
      if (this.name === 'sources' && globalThis.__rejectGisWrite) {
        globalThis.__gisWriteRejected = true;
        throw new DOMException('Test GIS storage quota', 'QuotaExceededError');
      }
      return put.apply(this, args);
    };
  });
  await importFile(rejectedPath);
  await page.waitForFunction(() => globalThis.__gisWriteRejected);
  await clickTarget(page, 'Save settings backup');
  await page.waitForFunction(() => globalThis.__settingsDownloads.length === 2);
  assert.ok(await page.evaluate(({largeGis, rejectedGis}) => {
    const sources = JSON.parse(globalThis.__settingsDownloads[1]).settings.web_files;
    return largeGis in sources && !(rejectedGis in sources);
  }, {largeGis, rejectedGis}), 'an aborted GIS write preserves the active settings');
  report.checks.push('An IndexedDB write failure preserves previous settings and GIS sources');
  assert.deepEqual(report.errors, []);
} catch (error) {
  report.failure = error.stack ?? String(error);
  await page.screenshot({path: path.join(output, 'failure.png')}).catch(() => {});
  throw error;
} finally {
  await fs.writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  await browser.close();
  await fs.rm(temporary, {recursive: true, force: true});
}
console.log(JSON.stringify(report, null, 2));
