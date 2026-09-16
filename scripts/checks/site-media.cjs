const { chromium } = require('playwright');
const assert = require('node:assert/strict');

(async () => {
  const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    await page.goto('http://127.0.0.1:8095/');
    assert.equal(await page.locator('.guided-media').count(), 6);
    for (const image of await page.locator('.guided-media img').all()) {
      await image.scrollIntoViewIfNeeded();
      await image.evaluate(element => element.decode());
      assert(await image.evaluate(element => element.complete && element.naturalWidth > 0));
    }
    const demo = page.locator('.guided-media').first();
    await demo.locator('[data-action="toggle"]').click();
    assert.equal(await demo.locator('[data-action="toggle"]').textContent(), 'Play');
    await demo.locator('[data-action="replay"]').click();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.reload();
    assert(await page.locator('.guided-media').first().locator('img').isVisible());
    await page.setViewportSize({ width: 390, height: 844 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    console.log('Guided media, controls, reduced motion, and phone layout pass.');
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
