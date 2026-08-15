// Load the built browser bundle in a real headless browser and fail if it does not come up.
//
// `cargo check --target wasm32` cannot see a runtime panic — which is how 0.6.0 shipped a web
// build that compiled and then died on its first `Instant::now()`. This is the cheapest thing
// that would have caught it: serve web/, open it, wait for index.html's own success signal (the
// #boot overlay is removed only after `start()` resolves), and treat any page error, wasm panic,
// or console error as a failure.
//
// ponytail: no test framework, no screenshot diffing — this answers one question, "does it
// boot", and the golden-image test already covers what the pipeline draws.
import puppeteer from "puppeteer";

const args = process.argv.slice(2);
// Only meaningful against a deployed origin, where `/proxy/` is real and volumes actually arrive.
const expectLoop = args.includes("--expect-loop");
const url = args.find((a) => !a.startsWith("--")) ?? "http://127.0.0.1:8080/";
const errors = [];

// A boot that takes minutes is a regression even though it eventually succeeds. Generous because
// this is SwiftShader on a shared runner — a healthy boot is 10–20 s, and real breakage is minutes.
const BOOT_BUDGET_MS = 45_000;

const browser = await puppeteer.launch({
  headless: true,
  // CI runners have no GPU: SwiftShader is the WebGL2 implementation, and recent Chrome hides it
  // behind this flag. Without it the canvas never gets a context and the app fails for a reason
  // that has nothing to do with our code.
  args: [
    "--no-sandbox",
    "--enable-unsafe-swiftshader",
    "--use-gl=angle",
    "--use-angle=swiftshader",
  ],
});
const page = await browser.newPage();
page.on("pageerror", (e) => errors.push(`page error: ${e.message}`));
let loopStarted = false;
page.on("console", (m) => {
  const text = m.text();
  // A Rust panic reaches the console as a warning or a log depending on the panic hook.
  if (/panicked at|RuntimeError: unreachable/.test(text)) errors.push(`panic: ${text}`);
  // The app logs this once, from `sync_timeline`, when the opening loop starts playing.
  if (/loop: playing \d+\/\d+ frames/.test(text)) {
    loopStarted = true;
    console.log(`(loop) ${text}`);
  }
});

// Distinct archived volumes fetched. One is the live frame; a loop needs the tail behind it, and
// counting URLs rather than requests keeps a retry from reading as progress.
const volumes = new Set();
page.on("request", (r) => {
  const u = r.url();
  if (/unidata-nexrad-level2\.s3\.amazonaws\.com/.test(u) && !u.includes("list-type=")) {
    volumes.add(u);
  }
});
// Failed data fetches are not a failure: a static file server has no `/proxy/…`, and the app is
// supposed to survive a feed being unreachable. Only the app's own crashes count.
page.on("requestfailed", (r) => console.log(`(offline, ignored) ${r.url()}`));

let ok = true;
const startedAt = Date.now();
try {
  // `domcontentloaded`, not `load`: the boot script is a module with top-level await, so `load`
  // does not fire until the app has already started — waiting for it would fold the entire boot
  // into `goto` and leave the budget below measuring nothing.
  await page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 });
  // index.html removes #boot only once init() and start() have both resolved; if start throws it
  // rewrites the overlay instead, so this waits for the one unambiguous success signal.
  await page.waitForFunction(() => !document.getElementById("boot"), { timeout: 90_000 });
  const bootMs = Date.now() - startedAt;
  console.log(`web smoke: booted in ${(bootMs / 1000).toFixed(1)}s`);
  if (bootMs > BOOT_BUDGET_MS) {
    errors.push(`boot took ${(bootMs / 1000).toFixed(1)}s, over the ${BOOT_BUDGET_MS / 1000}s budget`);
  }
  const size = await page.$eval("#hookecho", (c) => [c.width, c.height]);
  if (!size[0] || !size[1]) {
    errors.push(`canvas has no backing store: ${size.join("x")}`);
  }

  if (expectLoop) {
    // The demo is supposed to open *playing* the last few volumes. Both halves are checked: the
    // app said it started a loop, and the frames behind the live one were really fetched.
    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline && !(loopStarted && volumes.size >= 3)) {
      await new Promise((r) => setTimeout(r, 1_000));
    }
    if (!loopStarted) errors.push("the loop never started playing");
    if (volumes.size < 3) {
      errors.push(`only ${volumes.size} archived volumes fetched, expected at least 3`);
    }
  }
} catch (e) {
  const boot = await page.$eval("#boot", (el) => el.textContent).catch(() => null);
  errors.push(`did not start: ${e.message}${boot ? ` — overlay says: ${boot}` : ""}`);
}

if (errors.length) {
  ok = false;
  for (const e of errors) console.error(`::error::${e}`);
}
await browser.close();
console.log(ok ? "web smoke: the app booted" : "web smoke: FAILED");
process.exit(ok ? 0 : 1);
