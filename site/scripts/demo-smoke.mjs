import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { spawn } from "node:child_process";
import { setTimeout as wait } from "node:timers/promises";
import { chromium } from "playwright";

const url = "http://127.0.0.1:4323";
const server = spawn("npm", ["run", "dev", "--", "--host", "127.0.0.1", "--port", "4323"], { stdio: "ignore" });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 60; i++) {
    try { if ((await fetch(url)).ok) { ready = true; break; } } catch {}
    await wait(250);
  }
  assert.ok(ready, "Astro preview did not start");
  browser = await chromium.launch({ executablePath: existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined, args: ["--no-sandbox"] });

  const desktop = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await desktop.goto(url);
  assert.equal(await desktop.locator('[data-explore-analyst]').getAttribute("href"), "#analyst");
  await desktop.locator('[data-explore-analyst]').click();
  assert.equal(await desktop.locator('[data-mode="analyst"]').getAttribute("aria-pressed"), "true");
  assert.ok(await desktop.locator('[data-stage="four"]').isVisible());
  await desktop.locator('#analyst [data-action="play"]').click();
  assert.equal(await desktop.locator('#analyst [data-action="play"]').textContent(), "Play");
  await desktop.locator('#analyst [data-action="replay"]').click();
  await desktop.waitForFunction(() => document.querySelector('#analyst [data-action="play"]')?.textContent === "Pause");
  assert.equal(await desktop.locator('#analyst [data-action="play"]').textContent(), "Pause");
  await desktop.locator('#analyst [data-action="play"]').click();
  await desktop.locator('[data-stage="four"] video').evaluate((video) => {
    video.currentTime = 2;
    video.dispatchEvent(new Event("timeupdate"));
  });
  await desktop.locator('[data-mode="radar"]').focus();
  await desktop.keyboard.press("Enter");
  assert.ok(await desktop.locator('[data-stage="single"]').isVisible());
  await desktop.locator('#analyst [data-action="play"]').click();
  await desktop.waitForFunction(() => document.querySelector('[data-stage="single"] video')?.currentTime > 1.7);
  await desktop.locator('#analyst [data-action="play"]').click();
  await desktop.locator('[data-feature="2"]').click();
  assert.equal(await desktop.locator('[data-feature="2"]').getAttribute("aria-pressed"), "true");
  assert.ok((await desktop.locator('[data-feature-image]').getAttribute("src")).includes("mrms"));
  assert.equal(await desktop.evaluate(() => document.documentElement.scrollWidth - innerWidth), 0);
  await desktop.close();

  const mobile = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await mobile.goto(url);
  await mobile.locator('[data-explore-analyst]').click();
  await mobile.locator('[data-product="1"]').click();
  assert.ok(await mobile.locator('[data-product-frame="1"]').isVisible());
  const stage = mobile.locator('[data-stage="mobile"]');
  const box = await stage.boundingBox();
  await mobile.evaluate(({ x, y }) => {
    const target = document.querySelector('[data-stage="mobile"]');
    target.dispatchEvent(new TouchEvent("touchstart", { changedTouches: [new Touch({ identifier: 1, target, clientX: x + 200, clientY: y + 100 })] }));
    target.dispatchEvent(new TouchEvent("touchend", { changedTouches: [new Touch({ identifier: 1, target, clientX: x + 100, clientY: y + 100 })] }));
  }, { x: box.x, y: box.y });
  assert.ok(await mobile.locator('[data-product-frame="2"]').isVisible());
  assert.equal(await mobile.evaluate(() => document.documentElement.scrollWidth - innerWidth), 0);
  await mobile.close();

  const reduced = await browser.newPage({ viewport: { width: 390, height: 844 }, reducedMotion: "reduce" });
  await reduced.goto(url);
  await reduced.locator('[data-explore-analyst]').click();
  assert.ok(await reduced.locator('[data-product-frame="0"] img').isVisible());
  assert.ok(!(await reduced.locator('[data-product-frame="0"] video').isVisible()));
  await reduced.close();

  const noScript = await browser.newPage({ javaScriptEnabled: false });
  await noScript.goto(url);
  assert.ok(await noScript.locator('[data-stage="single"] noscript img').isVisible());
  assert.equal(await noScript.locator('.hero .btn-primary').getAttribute('href'), '/go/web/hero');
  await noScript.close();

  const failedMedia = await browser.newPage();
  await failedMedia.route(/clinton-.*\.mp4/, (route) => route.abort());
  await failedMedia.goto(url);
  await failedMedia.locator('[data-explore-analyst]').click();
  assert.ok(await failedMedia.locator('[data-stage="four"] img').isVisible());
  assert.equal(await failedMedia.locator('.hero .btn-primary').getAttribute('href'), '/go/web/hero');
  await failedMedia.close();
  console.log("Analyst demo smoke check passed");
} finally {
  await browser?.close();
  server.kill("SIGTERM");
}
