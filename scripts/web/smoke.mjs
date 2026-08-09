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

const url = process.argv[2] ?? "http://127.0.0.1:8080/";
const errors = [];

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
page.on("console", (m) => {
  const text = m.text();
  // A Rust panic reaches the console as a warning or a log depending on the panic hook.
  if (/panicked at|RuntimeError: unreachable/.test(text)) errors.push(`panic: ${text}`);
});
// Failed data fetches are not a failure: a static file server has no `/proxy/…`, and the app is
// supposed to survive a feed being unreachable. Only the app's own crashes count.
page.on("requestfailed", (r) => console.log(`(offline, ignored) ${r.url()}`));

let ok = true;
try {
  await page.goto(url, { waitUntil: "load", timeout: 60_000 });
  // index.html removes #boot only once init() and start() have both resolved; if start throws it
  // rewrites the overlay instead, so this waits for the one unambiguous success signal.
  await page.waitForFunction(() => !document.getElementById("boot"), { timeout: 90_000 });
  const size = await page.$eval("#hookecho", (c) => [c.width, c.height]);
  if (!size[0] || !size[1]) {
    errors.push(`canvas has no backing store: ${size.join("x")}`);
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
