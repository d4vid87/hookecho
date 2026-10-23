// Run against the local Astro server: node scripts/forecast-smoke.mjs [URL]
import { chromium } from 'playwright';
import assert from 'node:assert/strict';

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const now = Date.parse('2026-12-31T23:00:00-06:00'), chances = [0, null, 27, 80, 100, 101];
  await page.clock.setFixedTime(new Date(now));
  const periods = Array.from({ length: 24 }, (_, i) => ({
    name: i ? `${['Friday','Saturday','Sunday','Monday','Tuesday','Wednesday','Thursday'][Math.floor((i-1)/2)%7]}${i % 2 ? '' : ' Night'}` : 'Tonight',
    startTime: new Date(now + (i - 1) * 3600000).toISOString(),
    endTime: new Date(now + (i + 1) * 3600000).toISOString(),
    temperature: 79 - i, temperatureUnit: 'F', isDaytime: i % 2 === 1,
    shortForecast: i % 2 ? 'Chance Rain Showers' : 'Mostly Sunny',
    detailedForecast: 'Forecast details remain available.', windSpeed: '6 mph',
    probabilityOfPrecipitation: { value: chances[i % chances.length] },
  }));
  const dailyPeriods = periods.slice(0,14).map((p,i) => ({
    ...p,
    startTime: new Date(now - 5*3600000 + i*12*3600000).toISOString(),
    endTime: new Date(now + 7*3600000 + i*12*3600000).toISOString(),
  }));
  await page.route('https://app.hookecho.io/**', r => r.fulfill({ body: '' }));
  await page.route('https://api.weather.gov/**', r => {
    const url = r.request().url();
    const body = url.includes('/points/') ? { properties: { forecastHourly: 'https://api.weather.gov/test/hourly', forecast: 'https://api.weather.gov/test/daily', timeZone: 'America/Chicago' } }
      : url.includes('/alerts/') ? { features: [] }
      : { properties: { periods: url.endsWith('/daily') ? dailyPeriods : periods, updateTime: new Date(now).toISOString() } };
    return r.fulfill({ json: body });
  });
  await page.goto(process.argv[2] || 'http://127.0.0.1:4345/');
  await page.locator('.hour').last().waitFor();
  assert.equal(await page.locator('.hour').count(), 24);
  assert.equal(await page.locator('.days details').count(), 8);
  assert.equal(await page.locator('.period-date').count(), 8);
  assert.equal(await page.locator('.period-date').first().textContent(), 'Dec 31');
  assert.equal(await page.locator('.period-date').nth(1).textContent(), 'Jan 1');
  assert.equal(await page.locator('.period-date').first().getAttribute('datetime'), dailyPeriods[0].startTime);
  assert.deepEqual(await page.locator('.day-entry').evaluateAll(entries => entries.map(e => e.querySelectorAll('.day-period').length)), [1,2,2,2,2,2,2,1]);
  assert.equal(new Set(await page.locator('.period-date').allTextContents()).size, 8);
  assert.equal(await page.locator('.day-entry > p').count(), 14);
  assert.deepEqual(await page.locator('.period-phase-label').allTextContents(), Array.from({length:14}, (_,i) => i % 2 ? 'Day' : 'Night'));
  assert.deepEqual(await page.locator('.period-extreme').allTextContents(), Array.from({length:14}, (_,i) => i % 2 ? 'High' : 'Low'));
  assert.equal(await page.locator('.day-period').first().getAttribute('data-daytime'), 'false');
  assert.deepEqual(await page.locator('.hour .rain-value').allTextContents(), Array.from({ length: 24 }, (_, i) => ['0%', '—', '27%', '80%', '100%', '—'][i % 6]));
  assert.equal(await page.locator('.hour .rain-fill').nth(4).evaluate(e => e.style.width), '100%');
  assert.equal(await page.locator('.hour .rain-fill').nth(5).evaluate(e => e.style.width), '0%');
  await page.locator('.days summary').first().focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('.days details').first().getAttribute('open'), '');
  await page.locator('[data-unit="C"]').click();
  assert.equal(await page.locator('.hour strong').first().textContent(), '26°');
  await page.locator('.days details').first().evaluate(e => e.open = false);
  await page.locator('#week').evaluate(e => window.scrollTo(0, e.getBoundingClientRect().top + scrollY - 90));
  await page.screenshot({ path: '/tmp/hookecho-forecast-desktop.png' });
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert.ok(await page.locator('.period-rain').first().isVisible());
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    assert.ok(await page.locator('.hours').evaluate(e => e.scrollWidth > e.clientWidth));
    await page.locator('#week').scrollIntoViewIfNeeded();
    await page.screenshot({ path: `/tmp/hookecho-forecast-${width}.png` });
  }
  await page.emulateMedia({ reducedMotion: 'reduce' });
  assert.ok(await page.locator('.period-phase').first().evaluate(e => parseFloat(getComputedStyle(e).transitionDuration) <= .001));
  dailyPeriods[0].isDaytime = true;
  await page.reload();
  await page.locator('.period-phase-label').first().waitFor();
  assert.equal(await page.locator('.period-phase-label').first().textContent(), 'Day');
  assert.equal(await page.locator('.period-extreme').first().textContent(), 'High');
  assert.equal(await page.locator('.day-period').first().getAttribute('data-daytime'), 'true');
  assert.equal(await page.locator('.day-entry').count(), 8);
  console.log('Forecast checks passed: day/night semantics and shading, precipitation, keyboard details, units, desktop and mobile layouts.');
} finally {
  await browser.close();
}
