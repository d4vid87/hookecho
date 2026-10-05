// Capture current HookEcho UI from real archive and live radar scenes for site media.
// Run from the repository root after installing site dependencies:
//   node scripts/shots/capture-web-showcase.cjs /tmp/hookecho-showcase
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('../../site/node_modules/playwright');

const out = path.resolve(process.argv[2] || '/tmp/hookecho-showcase');
const archive = 'https://app.hookecho.io/#goto=KTLX,-97.36,35.40,9.9,2013-05-20T20:15:00Z';
const live = 'https://app.hookecho.io/#goto=KBMX,-86.6,32.4,6.8,thr:16,bm:dark';

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
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
  try {
    await page.goto(archive, { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(16000);
    await frames(page, 'radar', 9, async (p) => p.keyboard.press('ArrowRight'));

    // The same archived scan in the current four-pane Analyst view.
    await page.mouse.click(90, 216);
    await page.waitForTimeout(8000);
    await frames(page, 'analyst', 6, async (p) => p.keyboard.press('ArrowRight'));

    await page.goto('about:blank');
    await page.goto(live, { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(16000);
    await page.mouse.click(90, 113); // Radar view, after the Analyst recording.
    await page.waitForTimeout(1500);
    await page.mouse.click(100, 430);
    await page.waitForTimeout(2500);
    const cells = [[1045, 312], [1170, 312], [935, 365], [1045, 365], [1170, 365], [935, 312]];
    await frames(page, 'attributes', cells.length, async (p, i) => p.mouse.click(...cells[i + 1]));

    await page.goto('about:blank');
    await page.goto(live, { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(16000);
    await page.mouse.click(90, 113);
    await page.waitForTimeout(1200);
    await page.mouse.click(1237, 106); // Close the persistent Storm dock.
    await page.mouse.click(90, 183);
    await page.waitForTimeout(14000);
    await frames(page, 'national', 5, async (p) => p.keyboard.press('ArrowRight'));
  } finally {
    await browser.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
