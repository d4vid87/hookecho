// Run against the local Astro server: node scripts/forecast-smoke.mjs [URL]
import { chromium } from 'playwright';
import assert from 'node:assert/strict';

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const now = Date.now(), chances = [0, null, 27, 80, 100, 101];
  const periods = Array.from({ length: 24 }, (_, i) => ({
    name: i ? `Friday${i % 2 ? ' Night' : ''}` : 'This Afternoon',
    startTime: new Date(now + (i - 1) * 3600000).toISOString(),
    endTime: new Date(now + (i + 1) * 3600000).toISOString(),
    temperature: 79 - i, temperatureUnit: 'F', isDaytime: i % 2 === 0,
    shortForecast: i % 2 ? 'Chance Rain Showers' : 'Mostly Sunny',
    detailedForecast: 'Forecast details remain available.', windSpeed: '6 mph',
    probabilityOfPrecipitation: { value: chances[i % chances.length] },
  }));
  await page.route('https://app.hookecho.io/**', r => r.fulfill({ body: '' }));
  await page.route('https://api.weather.gov/**', r => {
    const url = r.request().url();
    const body = url.includes('/points/') ? { properties: { forecastHourly: 'https://api.weather.gov/test/hourly', forecast: 'https://api.weather.gov/test/daily', timeZone: 'America/Chicago' } }
      : url.includes('/alerts/') ? { features: [] }
      : { properties: { periods, updateTime: new Date(now).toISOString() } };
    return r.fulfill({ json: body });
  });
  await page.goto(process.argv[2] || 'http://127.0.0.1:4345/');
  await page.locator('.hour').last().waitFor();
  assert.equal(await page.locator('.hour').count(), 24);
  assert.equal(await page.locator('.days details').count(), 14);
  assert.deepEqual(await page.locator('.hour .rain-value').allTextContents(), Array.from({ length: 24 }, (_, i) => ['0%', '—', '27%', '80%', '100%', '—'][i % 6]));
  assert.equal(await page.locator('.hour .rain-fill').nth(4).evaluate(e => e.style.width), '100%');
  assert.equal(await page.locator('.hour .rain-fill').nth(5).evaluate(e => e.style.width), '0%');
  await page.locator('.days summary').first().focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('.days details').first().getAttribute('open'), '');
  await page.locator('[data-unit="C"]').click();
  assert.equal(await page.locator('.hour strong').first().textContent(), '26°');
  await page.locator('#forecast').scrollIntoViewIfNeeded();
  await page.screenshot({ path: '/tmp/hookecho-forecast-desktop.png' });
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert.ok(await page.locator('.period-rain').first().isVisible());
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    assert.ok(await page.locator('.hours').evaluate(e => e.scrollWidth > e.clientWidth));
    await page.locator('#week').scrollIntoViewIfNeeded();
    await page.screenshot({ path: `/tmp/hookecho-forecast-${width}.png` });
  }
  console.log('Forecast checks passed: precipitation edge cases, keyboard details, units, desktop and mobile layouts.');
} finally {
  await browser.close();
}
