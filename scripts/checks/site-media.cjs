// Serve the built site on port 8095. Requires Playwright and system Chromium.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');

(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    for (const path of ['/', '/stormdesk/']) {
      await page.goto('http://127.0.0.1:8095' + path);
      for (const image of await page.locator('.demo-media img').all()) {
        await image.scrollIntoViewIfNeeded();
        await image.evaluate(element => element.decode());
        assert(await image.evaluate(element => element.complete && element.naturalWidth > 0));
      }
      const button = page.locator('.demo-pause').first();
      const image = page.locator('.demo-media img').first();
      await button.click();
      assert((await image.getAttribute('src')).endsWith('.webp'));
      await button.click();
      assert((await image.getAttribute('src')).endsWith('.gif'));
    }
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.reload();
    assert(await page.locator('.demo-media img').first().evaluate(element => element.currentSrc.endsWith('.webp')));
    await page.setViewportSize({ width: 390, height: 844 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    console.log('Both product animations, controls, reduced-motion posters, and mobile layout pass.');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
