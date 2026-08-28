// Open the lite viewer in a real browser and fail if it does not paint a radar loop.
//
// Sibling of smoke.mjs, for the page that has none of that one's machinery: no WebGL, no boot
// overlay to watch disappear. The success signal is the toolbar's own timestamp, which only
// stops saying "Loading…" once a scan is decoded, plus non-empty pixels on the radar canvas.
import puppeteer from "puppeteer";

const url = process.argv[2] ?? "http://127.0.0.1:8788/lite/";
const errors = [];
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
await page.setViewport({ width: 900, height: 700 });
page.on("pageerror", (e) => errors.push(`page error: ${e.message}`));
page.on("console", (m) => {
  if (/panicked at|RuntimeError: unreachable/.test(m.text())) errors.push(`panic: ${m.text()}`);
});

await page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 });
try {
  await page.waitForFunction(
    () => {
      const t = document.getElementById("time")?.textContent ?? "";
      return /\d+\/\d+/.test(t);
    },
    { timeout: 60_000 },
  );
} catch {
  errors.push(`no frames: toolbar still reads ${JSON.stringify(await page.$eval("#time", (e) => e.textContent))}`);
}

// Painted, not merely present: a fully transparent canvas is what a broken projection looks like.
const painted = await page.evaluate(() => {
  const c = document.getElementById("radar");
  const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
  let n = 0;
  for (let i = 3; i < d.length; i += 4) if (d[i] > 0) n++;
  return n;
});
console.log(`lite: ${painted} painted pixels, toolbar ${await page.$eval("#time", (e) => e.textContent)}`);
if (painted === 0) errors.push("the radar canvas is empty");

await browser.close();
if (errors.length) {
  for (const e of errors) console.error(e);
  process.exit(1);
}
console.log("lite viewer OK");
