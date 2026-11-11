// Capture the actual shipped web UI with real upstream weather data. No fixtures or painted UI.
// HOOKECHO_BASE_URL=http://127.0.0.1:8767 node scripts/shots/capture-web-showcase.cjs [output] [group]
const fs = require('node:fs');
const path = require('node:path');
const puppeteer = require('puppeteer');
const assert = require('node:assert/strict');
const out = path.resolve(process.argv[2] || '/tmp/hookecho-showcase');
const group = process.argv[3] || 'all';
const base = (process.env.HOOKECHO_BASE_URL || 'https://app.hookecho.io').replace(/\/$/, '');
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const scenes = {
  moore: 'KTLX,-97.36,35.40,9.9,2013-05-20T20:15:00Z',
  tuscaloosa: 'KBMX,-87.55,33.20,9.3,2011-04-27T22:10:00Z',
  elreno: 'KTLX,-97.99,35.55,10.4,2013-05-31T23:10:00Z',
  mayfield: 'KPAH,-88.64,36.74,9.3,2021-12-11T03:30:00Z',
  ian: 'KTBW,-82.05,26.60,7.6,2022-09-28T19:00:00Z',
  live: process.env.HOOKECHO_LIVE_SCENE || 'KILX,-89.0,40.5,8.2,thr:16,bm:dark',
  national: 'KILX,-95.0,38.2,5.0,thr:16,bm:dark',
};
fs.mkdirSync(out, {recursive: true});
const report = {capturedAt: new Date().toISOString(), base, viewport: {width: 1600, height: 1000}, scenes: {}, failures: []};
let browser, page, clickTarget;
async function open(scene) {
  if (page) await page.close();
  page = await browser.newPage();
  await page.setViewport({...report.viewport, deviceScaleFactor: 1});
  await page.setBypassServiceWorker(true);
  const {installUiTargets} = await import('../web/fixtures/ui.mjs');
  await installUiTargets(page);
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    localStorage.clear();
    localStorage.setItem('hookecho-settings', JSON.stringify({setup_done: true, seeded_workspaces: true,
      default_site: 'KTLX', radar_workspace: true, reduce_motion: true, ui_scale: 1,
      hints_seen: ['info_links', 'panes'],
      overlays_on: ['Alerts', 'RadarSites'], show_warning_summary: false, live_loop_frames: 10}));
  });
  page.on('pageerror', error => report.failures.push(error.message));
  await page.goto(`${base}/#goto=${scene}`, {waitUntil: 'domcontentloaded'});
  await page.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  // The checkpoint only receives a scan timestamp after a real volume has loaded.
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.panes[0]?.time,
    {timeout: 90000});
  await wait(16000); // Allow map tiles, archived warning geometry and neighbour scans to finish.
  await page.evaluate(() => document.getElementById('dismiss-install')?.click());
  await page.mouse.move(1550, 800);
}
async function command(label) {
  await page.keyboard.down('Control'); await page.keyboard.press('k'); await page.keyboard.up('Control');
  await wait(600); await page.keyboard.type(label, {delay: 20}); await wait(700);
  await page.keyboard.press('Enter'); await wait(1800);
}
async function dismissPanel() {
  const state = await page.evaluate(() => ({
    close: !!globalThis.__hookechoUiTargets?.controls['Close menu']?.length,
    panel: JSON.parse(localStorage.getItem('hookecho.recovery.v1'))?.session.workspace.chrome?.panel_open,
  }));
  if (state.close) await clickTarget(page, 'Close menu');
  else if (state.panel) await page.keyboard.press('Escape');
  await wait(1200);
}
async function capture(name, settle = 1800) {
  await page.mouse.move(name.endsWith('/features') ? 125 : 1, name.endsWith('/features') ? 105 : 1); await wait(settle);
  const file = path.join(out, `${name}.png`); fs.mkdirSync(path.dirname(file), {recursive: true});
  await page.screenshot({path: file});
  report.scenes[name] = await page.evaluate(() => ({hash: location.hash,
    checkpoint: JSON.parse(localStorage.getItem('hookecho.recovery.v1')),
    controls: Object.keys(globalThis.__hookechoUiTargets?.controls || {})}));
  const panes = report.scenes[name].checkpoint?.session.workspace.panes || [];
  const product = name.match(/^product-([^/]+)\//)?.[1];
  const moments = {reflectivity: 'Reflectivity', velocity: 'Velocity', correlation: 'CorrelationCoefficient', differential: 'DifferentialReflectivity'};
  if (product) { assert.equal(panes.length, 1); assert.equal(panes[0].moment, moments[product]); }
  if (name.startsWith('analyst/')) assert.deepEqual(panes.map(p => p.moment), Object.values(moments));
  if (name === 'national-to-local/four-tilts') {
    assert.equal(panes.length, 4, 'four elevation panes loaded');
    assert(panes.every(p => p.moment === 'Reflectivity'), 'four tilts retain reflectivity');
    assert.equal(new Set(panes.map(p => p.tilt)).size, 4, 'each pane has a distinct elevation');
  }
  console.log(`captured ${name}`);
  fs.writeFileSync(path.join(out, `manifest-${group}.json`), JSON.stringify(report, null, 2));
}
async function frames(name, count = 8, action, interval = 2500, settle = 1800) {
  fs.rmSync(path.join(out, name), {recursive: true, force: true});
  fs.mkdirSync(path.join(out, name), {recursive: true});
  for (let i = 0; i < count; i++) {
    await capture(`${name}/${String(i).padStart(2, '0')}`, settle);
    if (action) await action(i); else await page.keyboard.press('ArrowRight');
    await wait(interval);
  }
}
async function run(name, task) {
  const only = process.env.HOOKECHO_CAPTURE_ONLY?.split(',');
  if (only && !only.includes(name)) return;
  try { await task(); } catch (error) {
    console.error(name, error.message); report.failures.push(`${name}: ${error.message}`);
    if (page) await capture(`failure-${name}`).catch(() => {});
  }
}
(async () => {
  ({clickTarget} = await import('../web/fixtures/ui.mjs'));
  browser = await puppeteer.launch({headless: true, executablePath: process.env.CHROME_BIN || '/usr/bin/chromium',
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']});
  try {
    if (group === 'all' || group === 'core') {
      await run('radar', async () => {
        await open(scenes.moore); await frames('radar', 9);
        await clickTarget(page, 'Full playback controls'); await capture('playback');
        await open(scenes.moore); await page.mouse.click(775, 518); await wait(1800); await capture('warning-detail');
      });
      await run('analyst', async () => {
        await open(scenes.moore); await command('Tornado analysis'); await wait(18000);
        await dismissPanel();
        await frames('analyst', 6); await capture('alltilts');
      });
      for (const [name, key] of [['reflectivity','1'], ['velocity','2'], ['correlation','7'], ['differential','4']]) {
        await run(`product-${name}`, async () => {
          await open(scenes.moore); await page.keyboard.press(key); await wait(3500); await frames(`product-${name}`, 5);
        });
      }
      await run('attributes', async () => {
        await open(scenes.live); await command('Storm attributes…'); await wait(8000); await dismissPanel();
        await capture('stormtable'); await frames('attributes', 6, async () => {});
      });
    }
    if (group === 'all' || group === 'tour') {
      await run('national', async () => {
        await open(scenes.national); await clickTarget(page, 'National mosaic (MRMS)'); await wait(20000);
        await frames('national', 6, async () => {}); await capture('national-to-local/mrms');
        await clickTarget(page, 'Open workspace menu'); await capture('national-to-local/features');
      });
      await run('spc', async () => {
        await open(scenes.national); await clickTarget(page, 'SPC convective outlook');
        await wait(1200); await page.mouse.click(205, 400); await wait(10000); await capture('national-to-local/spc');
      });
      await run('layouts', async () => {
        await open(scenes.moore); await clickTarget(page, 'Choose map layout and pane count');
        await capture('national-to-local/layouts');
        await open(scenes.moore); await command('Compare 4 tilts'); await wait(18000);
        await dismissPanel();
        await capture('national-to-local/four-tilts');
      });
      await run('sites', async () => {
        await open(scenes.live); await clickTarget(page, 'Select radar station'); await capture('site-selector');
      });
    }
    if (group === 'all' || group === 'gallery') {
      for (const [name, scene, key] of [['reflectivity','tuscaloosa','1'], ['velocity','elreno','2'], ['tropical','ian','1']]) {
        await run(name, async () => { await open(scenes[scene]); await page.keyboard.press(key); await wait(3000); await capture(name); });
      }
      for (const [name, scene, label] of [
        ['layers', 'mayfield', 'Layers panel'], ['products','moore',''],
        ['verify','moore','Warning verification…'], ['derived','mayfield','Hail signal (VIL density, derived)'],
        ['fronts','national','Surface fronts (H/L)'], ['hrrr','national','HRRR future radar'],
        ['mrms','national','National mosaic (MRMS)'], ['mosaic','live','Seamless mosaic (single-radar)'],
        ['qpe','live','Rain so far, 24 hours (QPE)'],
      ]) {
        await run(name, async () => {
          // Model grids and hourly QPE use their real valid hour. Pin radar to that
          // hour so the app's freshness check can display the field truthfully.
          let location = scenes[scene];
          if (name === 'hrrr' || name === 'qpe') {
            const hour = Math.floor(Date.now() / 3600000) * 3600000 - (name === 'qpe' ? 3600000 : 0);
            location = location.split(',').slice(0, 4).join(',') + ',' + new Date(hour).toISOString();
          }
          await open(location);
          if (name === 'layers') { await page.keyboard.press('l'); await wait(1800); }
          else if (label) { await command(label); await wait(['hrrr', 'qpe'].includes(name) ? 40000 : 18000);
            if (name !== 'verify') { await dismissPanel(); }
          }
          else { await page.keyboard.down('Control'); await page.keyboard.press('k'); await page.keyboard.up('Control'); await wait(500); await page.keyboard.type('hail'); }
          await capture(name);
        });
      }
      await run('alerts', async () => { await open(scenes.tuscaloosa); await page.mouse.click(916, 425); await wait(2500); await capture('alerts'); });
      await run('forecast', async () => {
        await open(scenes.live); await command('Tool: Point forecast'); await dismissPanel();
        await page.mouse.click(950, 430); await page.mouse.click(950, 430); await wait(12000); await capture('forecast');
      });
      await run('wind', async () => {
        await open(scenes.national); await command('Wind (animated)'); await wait(35000);
        await dismissPanel();
        await frames('wind', 12, async () => {}, 100, 0);
      });
    }
  } finally {
    fs.writeFileSync(path.join(out, `manifest-${group}.json`), JSON.stringify(report, null, 2));
    await browser.close();
  }
  if (report.failures.length) { console.error(report.failures); process.exitCode = 1; }
})();
