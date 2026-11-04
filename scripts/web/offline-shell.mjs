import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import path from 'node:path';

const root = path.resolve('web');
let failNavigation = false;
const types = {'.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.ttf': 'font/ttf', '.png': 'image/png', '.json': 'application/json', '.webmanifest': 'application/manifest+json'};
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, 'http://local.test').pathname;
  if (failNavigation && pathname === '/') {
    response.writeHead(503, {'content-type': 'text/html'}).end('<html>temporary outage</html>');
    return;
  }
  const file = path.resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
  if (!file.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, {'content-type': types[path.extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-cache'}).end(bytes);
  } catch { response.writeHead(404).end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${server.address().port}/`;
const browser = await puppeteer.launch({
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(), headless: true,
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
});
const errors = [];
async function open() {
  const page = await browser.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', {value: undefined});
    if (!localStorage.getItem('hookecho-settings')) localStorage.setItem('hookecho-settings', JSON.stringify({
      setup_done: true, seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly',
    }));
  });
  return page;
}
try {
  const online = await open();
  await online.goto(base, {waitUntil: 'load'});
  await online.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  await online.evaluate(() => navigator.serviceWorker.ready);
  await online.waitForFunction(() => !!navigator.serviceWorker.controller);
  const shellKeys = await online.evaluate(async () => {
    const name = (await caches.keys()).find(name => name.startsWith('shell-'));
    return (await (await caches.open(name)).keys()).map(request => new URL(request.url).pathname);
  });
  assert(shellKeys.some(path => path.includes('/snippets/')), 'inline modules must be persisted');
  failNavigation = true;
  await online.reload({waitUntil: 'load'});
  await online.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  assert(await online.$('#hookecho'), 'HTTP 503 must use the installed app');
  console.log('HTTP 503 preserves and boots the installed shell');
  const session = await online.createCDPSession();
  await session.send('Network.clearBrowserCache');
  await online.close();
  await new Promise(resolve => server.close(resolve));
  const offline = await open();
  await offline.setOfflineMode(true);
  await offline.goto(base, {waitUntil: 'load'});
  await offline.waitForFunction(() => !document.getElementById('boot'), {timeout: 90000});
  assert(await offline.$('#hookecho'));
  assert.equal(await offline.evaluate(() => typeof globalThis.__decodeVolume), 'function');
  assert.deepEqual(errors, []);
  console.log('Cold offline boot passed with HTTP cache cleared, local server stopped, and Cache Storage retained');
} finally {
  await browser.close();
  if (server.listening) await new Promise(resolve => server.close(resolve));
}
