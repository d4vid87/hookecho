// Cold-cache load measurement for the browser demo, in Chrome and Firefox.
//
//   node scripts/web/measure.mjs https://hookecho.pages.dev/
//
// smoke.mjs answers "does it boot"; this answers "how long until there is radar on screen", which
// is the number the web work is actually judged on. Every run gets a fresh profile and an
// explicitly disabled HTTP cache, so each one is a first-time visitor.
//
// Firefox needs puppeteer's own build (the system one speaks a different protocol version):
//   npx puppeteer browsers install firefox
//
// ponytail: medians of a handful of runs, printed for a human to read — no baseline file, no
// pass/fail. A load-time budget belongs in CI only once the numbers are stable enough that a red
// run means something; today the spread across runs is wider than most regressions would be.
import puppeteer from "puppeteer";

const URL_ = process.argv[2] ?? "https://hookecho.pages.dev/";
const RUNS = Number(process.env.RUNS ?? 5);

async function once(product) {
  const browser = await puppeteer.launch({
    browser: product,
    headless: true,
    // Headless has no GPU, so this is software rasterization. It penalizes drawing, not download
    // or decode — which is what is being measured here.
    args:
      product === "firefox"
        ? []
        : ["--no-sandbox", "--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader"],
  });
  const page = await browser.newPage();
  await page.setCacheEnabled(false);

  // The closest thing to "first radar paint" observable from outside the app: the page's decode
  // bridge resolving for the first time (radar is drawn the frame after). Installed as a setter so
  // it wraps index.html's own assignment whenever that happens.
  await page.evaluateOnNewDocument(() => {
    let real = null;
    Object.defineProperty(globalThis, "__decodeVolume", {
      configurable: true,
      get: () =>
        real === null
          ? undefined
          : (bytes) =>
              real(bytes).then((r) => {
                globalThis.__firstDecode ??= performance.now();
                return r;
              }),
      set: (v) => {
        real = v;
      },
    });
  });

  let loopAt = null;
  let firstVolumeAt = null;
  page.on("console", (m) => {
    if (loopAt === null && /loop: playing \d+\/\d+ frames/.test(m.text())) loopAt = Date.now();
  });
  page.on("response", (r) => {
    const u = r.url();
    // The proxied archive volume — what has to arrive before radar can be drawn at all.
    if (
      firstVolumeAt === null &&
      /unidata-nexrad-level2\.s3\.amazonaws\.com/.test(u) &&
      !u.includes("list-type=")
    ) {
      firstVolumeAt = Date.now();
    }
  });

  const wall0 = Date.now();
  // `domcontentloaded`, not `load`: the boot script is a module with top-level await, so `load`
  // does not fire until the app has already started.
  await page.goto(URL_, { waitUntil: "domcontentloaded", timeout: 120_000 });
  await page.waitForFunction(() => !document.getElementById("boot"), { timeout: 120_000 });
  const bootWall = Date.now();

  // Give the radar a chance to land, but never hang the measurement on it.
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline && (loopAt === null || firstVolumeAt === null)) {
    await new Promise((r) => setTimeout(r, 250));
  }

  const res = await page.evaluate(() => {
    const first = (re) => {
      const e = performance.getEntriesByType("resource").find((x) => re.test(x.name));
      return e ? { end: e.responseEnd, bytes: e.encodedBodySize || e.transferSize || 0 } : null;
    };
    return {
      wasm: first(/hookecho_bg-.*\.wasm/),
      geo: first(/geo\.json/),
      firstDecode: globalThis.__firstDecode ?? null,
    };
  });

  await browser.close();
  return {
    boot: bootWall - wall0,
    firstVolume: firstVolumeAt ? firstVolumeAt - wall0 : null,
    loop: loopAt ? loopAt - wall0 : null,
    wasmEnd: res.wasm?.end ?? null,
    // What the visitor actually downloads: the hosts serve brotli, so this is well under the
    // gzip figure build.sh gates on.
    wasmBytes: res.wasm?.bytes ?? 0,
    geoEnd: res.geo?.end ?? null,
    firstDecode: res.firstDecode,
  };
}

const fmt = (ms) => (ms === null ? "  n/a" : `${(ms / 1000).toFixed(2)}s`);
const median = (xs) => {
  const v = xs.filter((x) => x !== null).sort((a, b) => a - b);
  return v.length ? v[Math.floor(v.length / 2)] : null;
};

console.log(`${URL_}  (${RUNS} cold runs each)`);
for (const [name, product] of [
  ["Chrome", "chrome"],
  ["Firefox", "firefox"],
]) {
  const rows = [];
  for (let i = 0; i < RUNS; i++) {
    try {
      rows.push(await once(product));
    } catch (e) {
      console.log(`${name} run ${i + 1}: FAILED — ${e.message.split("\n")[0]}`);
    }
  }
  if (!rows.length) continue;
  const med = (k) => fmt(median(rows.map((r) => r[k])));
  console.log(`\n=== ${name} (${rows.length} runs, median) ===`);
  console.log(`  wasm downloaded    ${med("wasmEnd")}  (${(median(rows.map((r) => r.wasmBytes)) / 1e6).toFixed(2)} MB on the wire)`);
  console.log(`  geo.json           ${med("geoEnd")}`);
  console.log(`  app booted         ${med("boot")}`);
  console.log(`  first volume in    ${med("firstVolume")}`);
  console.log(`  first radar drawn  ${med("firstDecode")}`);
  console.log(`  loop playing       ${med("loop")}`);
  console.log(`  loop, every run:   ${rows.map((r) => fmt(r.loop).trim()).join(", ")}`);
}
