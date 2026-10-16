// Capture Radar Zeta UI from real archive and live radar scenes for site media.
// Run from the repository root after installing site dependencies:
//   node scripts/shots/capture-web-showcase.cjs /tmp/hookecho-showcase
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('../../site/node_modules/playwright');

const out = path.resolve(process.argv[2] || '/tmp/hookecho-showcase');
const archive = 'https://app.hookecho.io/#goto=KTLX,-97.36,35.40,9.9,2013-05-20T20:15:00Z';
const live = 'https://app.hookecho.io/#goto=KBMX,-86.6,32.4,6.8,thr:16,bm:dark';

async function palette(page, label) {
  await page.keyboard.press('Control+k');
  await page.waitForTimeout(500);
  await page.keyboard.type(label);
  await page.waitForTimeout(700);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(1200);
}

async function frames(page, name, count, action) {
  const dir = path.join(out, name);
  fs.mkdirSync(dir, { recursive: true });
  for (let i = 0; i < count; i++) {
    await page.screenshot({ path: path.join(dir, `${String(i).padStart(2, '0')}.png`) });
    if (i + 1 < count) {
      await action(page, i);
      await page.waitForTimeout(1300);
    }
  }
}

(async () => {
  const browser = await chromium.launch({
    headless: true,
    executablePath: '/usr/bin/chromium',
    args: ['--no-sandbox'],
  });
  let page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
  try {
    await page.goto(archive, { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(16000);
    await frames(page, 'radar', 9, async (p) => p.keyboard.press('ArrowRight'));

    // Open the archived Moore polygon for the warning-detail still. The larger
    // viewport keeps both the warning reader and the white warning border visible.
    const warningPage = await browser.newPage({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: 1 });
    await warningPage.goto(archive, { waitUntil: 'domcontentloaded' });
    await warningPage.waitForTimeout(16000);
    await warningPage.mouse.click(785, 526);
    await warningPage.waitForTimeout(1500);
    await warningPage.screenshot({ path: path.join(out, 'warning-detail.png') });
    await warningPage.close();

    // Keep the same archived scan and choose the four-pane Analyst workspace.
    await palette(page, '4 panes');
    await page.waitForTimeout(8000);
    await page.mouse.click(340, 420); // Correlation coefficient, lower-left pane.
    await page.mouse.click(650, 420); // Differential reflectivity, lower-right pane.
    await page.waitForTimeout(2500);
    await frames(page, 'analyst', 6, async (p) => p.keyboard.press('ArrowRight'));

    await page.close();
    page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
    await page.goto(live, { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(16000);
    await palette(page, 'Storm attributes…');
    await page.keyboard.press('Escape');
    await page.waitForTimeout(2500);
    await frames(page, 'attributes', 6, async (p, i) => {
      await p.mouse.click(i % 2 === 0 ? 954 : 1062, 315);
    });

    await page.close();
    page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
    await page.goto('https://app.hookecho.io/#goto=KTLX,-91.0,38.2,5.6,thr:16,bm:dark', { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(16000);
    await palette(page, 'National mosaic (MRMS)');
    await page.keyboard.press('Escape');
    await page.waitForTimeout(14000);
    await frames(page, 'national', 5, async (p) => p.keyboard.press('ArrowRight'));
  } finally {
    await browser.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
