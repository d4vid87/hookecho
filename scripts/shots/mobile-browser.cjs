// Current mobile web UI, captured independently from the native Android app.
// HOOKECHO_CAPTURE_ORIGIN=http://127.0.0.1:8767 node scripts/shots/mobile-browser.cjs
const { chromium } = require('../../site/node_modules/playwright');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');

const root = path.resolve(__dirname, '../..');
const shots = path.join(root, 'docs/shots/mobile');
const frames = '/tmp/hookecho-mobile-frames';
const origin = process.env.HOOKECHO_CAPTURE_ORIGIN || 'https://app.hookecho.io';
const scene = '#goto=KTLX,-97.36,35.40,9.9,2013-05-20T20:15:00Z,bm:dark';

(async () => {
  fs.mkdirSync(shots, { recursive: true });
  fs.mkdirSync(frames, { recursive: true });
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_BIN || '/usr/bin/chromium',
    args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 430, height: 932 }, deviceScaleFactor: 1 });
    const errors = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.goto(origin + '/' + scene, { waitUntil: 'domcontentloaded' });
    await page.locator('canvas#hookecho').waitFor({ state: 'visible' });
    // Historical scans and basemap tiles load independently of the canvas shell.
    await page.waitForTimeout(30000);
    assert.deepEqual(errors, [], 'Mobile app must start without an uncaught error');
    await page.screenshot({ path: path.join(shots, 'map.jpg'), type: 'jpeg', quality: 88 });
    fs.copyFileSync(path.join(shots, 'map.jpg'), path.join(shots, 'radar-mode.jpg'));

    for (const [name, x] of [['layers', 155], ['alerts', 255], ['more', 375]]) {
      await page.mouse.click(x, 890, { delay: 120 });
      await page.mouse.move(425, 200);
      await page.waitForTimeout(4500);
      await page.screenshot({ path: path.join(shots, `${name}.jpg`), type: 'jpeg', quality: 88 });
    }
    await page.mouse.click(390, 457, { delay: 120 }); // Close the Tools sheet before playback.
    await page.mouse.move(425, 200);
    await page.waitForTimeout(2000);

    const hashes = new Set();
    for (let i = 0; i < 10; i++) {
      const file = path.join(frames, `${String(i).padStart(2, '0')}.png`);
      await page.screenshot({ path: file });
      hashes.add(createHash('sha256').update(fs.readFileSync(file)).digest('hex'));
      if (i < 9) {
        await page.keyboard.press('ArrowRight');
        await page.waitForTimeout(2500);
      }
    }
    assert.ok(hashes.size > 1, 'Playback capture must contain changing frames');

    fs.writeFileSync(path.join(frames, 'capture.json'), JSON.stringify({
      capturedAt: new Date().toISOString(), platform: 'mobile web', viewport: { width: 430, height: 932 },
      origin, scene, frames: 10, distinctFrames: hashes.size, errors,
    }, null, 2) + '\n');
    console.log(`Captured mobile web screenshots and ${hashes.size} distinct frames at ${frames}`);
  } finally {
    await browser.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });

