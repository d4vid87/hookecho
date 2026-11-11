// Exercise the actual wasm GIF encoder in its dedicated worker, without automating radar capture.
import puppeteer from 'puppeteer';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fixtureRadar } from './fixtures/radar.mjs';

const output = path.resolve(process.env.HOOKECHO_EXPORT_OUTPUT ?? 'outputs/loop-export');
await fs.mkdir(output, { recursive: true });
await fs.rm(path.join(output, 'hookecho-loop.gif'), { force: true });
const report = { errors: [], checks: {} };
const browser = await puppeteer.launch({ headless: true,
  executablePath: process.env.CHROME_BIN ?? puppeteer.executablePath(),
  args: ['--no-sandbox', '--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'] });
try {
  const page = await browser.newPage();
  await page.setViewport({ width: 1050, height: 800 });
  await page.setBypassServiceWorker(true);
  await fixtureRadar(page, { vector: true });
  page.on('pageerror', error => report.errors.push(error.message));
  await page.evaluateOnNewDocument(() => {
    Object.defineProperty(navigator, 'gpu', { value: undefined });
    localStorage.clear();
    localStorage.setItem('hookecho-settings', JSON.stringify({ setup_done: true,
      seeded_workspaces: true, default_site: 'KDMX', radar_feed: 'ArchiveOnly', live_loop_frames: 1 }));
  });
  const session = await page.createCDPSession();
  await session.send('Page.setDownloadBehavior', { behavior: 'allow', downloadPath: output });
  await page.goto(process.env.HOOKECHO_BASE_URL ?? 'http://127.0.0.1:8765/', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => globalThis.__hookechoCreateExportWorker && !document.getElementById('boot'), { timeout: 90000 });
  report.checks.gif = await page.evaluate(async () => {
    const width = 384, height = 256, count = 8;
    const frames = Array.from({ length: count }, (_, index) => {
      const pixels = new Uint8Array(width * height * 4);
      for (let pixel = 0; pixel < width * height; pixel++) {
        pixels[pixel * 4 + (index % 2 ? 2 : 0)] = 255;
        pixels[pixel * 4 + 3] = 255;
      }
      return pixels;
    });
    let ticks = 0, animationFrames = 0, running = true;
    const timer = setInterval(() => ticks++, 1);
    const animate = () => { if (running) { animationFrames++; requestAnimationFrame(animate); } };
    requestAnimationFrame(animate);
    const progress = [];
    let message;
    try { message = await globalThis.__hookechoExportGif(frames, width, height, 125, done => progress.push(done)); }
    finally { clearInterval(timer); running = false; }
    const link = document.querySelector('#hookecho-video-save a');
    if (!link) throw new Error('GIF export did not offer a Save link');
    const blob = await fetch(link.href).then(response => response.blob());
    const decoder = new ImageDecoder({ data: await blob.arrayBuffer(), type: 'image/gif' });
    await decoder.tracks.ready;
    const decodedFrames = decoder.tracks.selectedTrack.frameCount;
    const colors = [];
    for (const frameIndex of [0, 1]) {
      const { image } = await decoder.decode({ frameIndex });
      const canvas = document.createElement('canvas');
      canvas.width = width; canvas.height = height;
      const context = canvas.getContext('2d');
      context.drawImage(image, 0, 0);
      colors.push(Array.from(context.getImageData(0, 0, 1, 1).data));
      image.close();
    }
    decoder.close();
    return { message, ticks, animationFrames, progress, decodedFrames, colors, bytes: blob.size, filename: link.download };
  });
  assert.equal(report.checks.gif.decodedFrames, 8);
  assert.deepEqual(report.checks.gif.progress, [1, 2, 3, 4, 5, 6, 7, 8]);
  assert(report.checks.gif.ticks > 0 && report.checks.gif.animationFrames > 0, 'the page must keep scheduling while the worker encodes');
  assert(report.checks.gif.colors[0][0] > 240 && report.checks.gif.colors[0][2] < 20, 'first source frame remains red');
  assert(report.checks.gif.colors[1][2] > 240 && report.checks.gif.colors[1][0] < 20, 'second source frame remains blue');
  assert.equal(report.checks.gif.filename, 'hookecho-loop.gif');
  await page.click('#hookecho-video-save a');
  const destination = path.join(output, 'hookecho-loop.gif');
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    if (await fs.stat(destination).then(() => true, () => false)) break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  const downloaded = await fs.readFile(destination);
  assert.equal(downloaded.subarray(0, 3).toString(), 'GIF');
  assert.equal(downloaded.length, report.checks.gif.bytes);
  report.checks.cancel = await page.evaluate(async () => {
    document.querySelector('#hookecho-video-save button').click();
    const pixels = Array.from({ length: 8 }, () => new Uint8Array(384 * 256 * 4));
    let promise;
    let completed = 0;
    promise = globalThis.__hookechoExportGif(pixels, 384, 256, 125, done => {
      completed = done;
      promise.cancel();
    });
    let error;
    try { await promise; } catch (failure) { error = failure.message; }
    await new Promise(resolve => setTimeout(resolve, 100));
    return { error, completed, hasDownload: Boolean(document.getElementById('hookecho-video-save')) };
  });
  assert.equal(report.checks.cancel.error, 'Loop export cancelled');
  assert(report.checks.cancel.completed >= 1, 'cancel an encoding job after its first real progress message');
  assert.equal(report.checks.cancel.hasDownload, false);
  assert.deepEqual(report.errors, []);
  await page.screenshot({ path: path.join(output, 'loop-export.png') });
} finally {
  await fs.writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  await browser.close();
}
console.log(JSON.stringify(report));
