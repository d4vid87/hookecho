import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('./video-export.js', import.meta.url), 'utf8');
function environment({ finishRecorder = true, fastFinishTimeout = false } = {}) {
  const panels = [];
  const workers = [];
  const tracks = [];
  class Element {
    constructor(tag) { this.tag = tag; this.children = []; this.style = {}; }
    append(...children) { this.children.push(...children); }
    addEventListener() {}
    remove() { panels.splice(panels.indexOf(this), 1); }
  }
  class Canvas extends Element {
    getContext() { return { putImageData() {}, fillRect() {} }; }
    captureStream() {
      const track = { stopped: false, requestFrame() {}, stop() { this.stopped = true; } };
      tracks.push(track);
      return { getVideoTracks: () => [track], getTracks: () => [track] };
    }
  }
  class Recorder {
    static isTypeSupported(mime) { return mime.startsWith('video/webm'); }
    constructor(stream, options) { this.mimeType = options.mimeType; this.state = 'inactive'; }
    start() { this.state = 'recording'; }
    stop() {
      this.state = 'inactive';
      this.ondataavailable({ data: new Blob([new Uint8Array(128)]) });
      if (finishRecorder) this.onstop();
    }
  }
  const globals = {
    Blob, Uint8Array, Uint8ClampedArray, clearTimeout,
    setTimeout: (callback, milliseconds) => setTimeout(callback, fastFinishTimeout && milliseconds === 10_000 ? 5 : milliseconds),
    HTMLCanvasElement: Canvas, MediaRecorder: Recorder,
    ImageData: class { constructor(bytes) { this.bytes = bytes; } },
    URL: { createObjectURL: () => 'blob:test', revokeObjectURL() {} },
    document: {
      createElement: tag => tag === 'canvas' ? new Canvas(tag) : new Element(tag),
      getElementById: id => panels.find(panel => panel.id === id),
      body: { append: panel => panels.push(panel) },
    },
    __hookechoCreateExportWorker: () => {
      const worker = { terminated: false, postMessage(message, transfer) { this.job = message; this.transfer = transfer; }, terminate() { this.terminated = true; } };
      workers.push(worker);
      return worker;
    },
  };
  vm.runInNewContext(source, globals);
  return { globals, panels, workers, tracks };
}

const frames = (count = 2) => Array.from({ length: count }, () => new Uint8Array(4 * 4 * 4));

test('GIF transfers pixels to its worker, reports progress, and offers a real Save link', async () => {
  const env = environment();
  const pixels = frames();
  const progress = [];
  const promise = env.globals.__hookechoExportGif(pixels, 4, 4, 100, done => progress.push(done));
  const worker = env.workers[0];
  assert.equal(worker.transfer.length, 2);
  assert.equal(worker.transfer[0], pixels[0].buffer);
  worker.onmessage({ data: { progress: 1 } });
  worker.onmessage({ data: { ok: new Uint8Array([71, 73, 70]).buffer } });
  assert.match(await promise, /GIF ready to save \(2 frames\)/);
  assert.deepEqual(progress, [1]);
  assert.equal(worker.terminated, true);
  assert.equal(pixels.length, 0);
  assert.equal(env.panels[0].children[1].download, 'hookecho-loop.gif');
});

test('GIF cancellation terminates the worker and never publishes late output', async () => {
  const env = environment();
  const progress = [];
  const promise = env.globals.__hookechoExportGif(frames(), 4, 4, 100, done => progress.push(done));
  const deliver = env.workers[0].onmessage;
  const rejected = assert.rejects(promise, /Loop export cancelled/);
  promise.cancel();
  deliver({ data: { progress: 1 } });
  deliver({ data: { ok: new ArrayBuffer(20) } });
  await rejected;
  assert.equal(env.workers[0].terminated, true);
  assert.equal(env.panels.length, 0);
  assert.deepEqual(progress, [], 'a cancelled job must not call the released Rust progress callback');
});

test('GIF failure cleans its worker and oversized loops never start one', async () => {
  const env = environment();
  const promise = env.globals.__hookechoExportGif(frames(), 4, 4, 100, () => {});
  env.workers[0].onerror({ message: 'worker failed' });
  await assert.rejects(promise, /worker failed/);
  assert.equal(env.workers[0].terminated, true);
  const large = Array(19).fill(new Uint8Array(960 * 960 * 4));
  await assert.rejects(env.globals.__hookechoExportGif(large, 960, 960, 100, () => {}), /memory limit/);
  assert.equal(env.workers.length, 1);
  assert.equal(env.panels.length, 0);
});

test('video yields to input and cancellation stops recording without a download', async () => {
  const env = environment();
  const progress = [];
  const pixels = frames();
  const promise = env.globals.__hookechoExportVideo(pixels, 4, 4, 1, done => progress.push(done));
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(progress, [1]);
  const rejected = assert.rejects(promise, /Loop export cancelled/);
  promise.cancel();
  await rejected;
  assert.equal(env.tracks[0].stopped, true);
  assert.equal(pixels.length, 0);
  assert.equal(env.panels.length, 0);
});

test('video preserves the recorder MIME extension and releases captured pixels', async () => {
  const env = environment();
  const pixels = frames(1);
  const progress = [];
  const result = await env.globals.__hookechoExportVideo(pixels, 4, 4, 15, done => progress.push(done));
  assert.match(result, /WEBM ready to save \(1 frames\)/);
  assert.deepEqual(progress, [1]);
  assert.equal(env.panels[0].children[1].download, 'hookecho-loop.webm');
  assert.equal(env.tracks[0].stopped, true);
  assert.equal(pixels.length, 0);
});

test('video cancellation interrupts a recorder that never emits its stop event', async () => {
  const env = environment({ finishRecorder: false });
  const promise = env.globals.__hookechoExportVideo(frames(1), 4, 4, 15, () => {});
  await new Promise(resolve => setTimeout(resolve, 150));
  const rejected = assert.rejects(promise, /Loop export cancelled/);
  promise.cancel();
  await rejected;
  assert.equal(env.tracks[0].stopped, true);
  assert.equal(env.panels.length, 0);
});

test('video stop has a deadline and never publishes incomplete output', async () => {
  const env = environment({ finishRecorder: false, fastFinishTimeout: true });
  await assert.rejects(env.globals.__hookechoExportVideo(frames(1), 4, 4, 15, () => {}), /recorder did not finish/);
  assert.equal(env.tracks[0].stopped, true);
  assert.equal(env.panels.length, 0);
});
