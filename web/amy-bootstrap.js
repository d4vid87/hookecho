const VERSION = "amy-medium-39ab474b";
const CACHE = `voice-${VERSION}`;
const MODEL_SHA256 = "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18";
const MODEL_BYTES = 63201294;
const MODEL = "en_US-amy-medium.onnx";
const VERIFIED = "hookecho-amy-verified.json";
const WORKER = "/amy-worker.js?protocol=2";
const INIT_TIMEOUT_MS = 60_000;

function status(text, progress = null) {
  globalThis.__hookechoAmyStatus = progress == null ? text : `${text} ${Math.round(progress * 100)}%`;
  document.documentElement.dataset.amyStatus = globalThis.__hookechoAmyStatus;
}

let worker;
let audio;
let audioUrl;
let nextJob = 0;

function clearAudio() {
  if (audio) {
    audio.pause();
    audio.removeAttribute("src");
    audio = null;
  }
  if (audioUrl) {
    URL.revokeObjectURL(audioUrl);
    audioUrl = null;
  }
  globalThis.__hookechoAmySpeaking = false;
}

globalThis.__hookechoAmySpeak = (text, volume) => {
  if (!worker || globalThis.__hookechoAmyStatus !== "Amy — ready") {
    throw new Error("Amy is not ready");
  }
  globalThis.__hookechoAmySpeaking = true;
  worker.postMessage({ type: "speak", id: ++nextJob, text, volume: Math.max(0, Math.min(1, volume)) });
};
globalThis.__hookechoAmyStop = () => {
  nextJob++;
  clearAudio();
};
globalThis.__hookechoAmyRetry = () => {
  if (globalThis.__hookechoAmyPreparing) return globalThis.__hookechoAmyPreparing;
  nextJob++;
  clearAudio();
  if (worker) worker.terminate();
  worker = null;
  return prepareAmy();
};

async function sha256(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function fileInfo(dir, name) {
  const file = await (await dir.getFileHandle(name)).getFile();
  return { size: file.size, lastModified: file.lastModified };
}

async function verifiedModel(dir) {
  try {
    const marker = await (await dir.getFileHandle(VERIFIED)).getFile();
    if (marker.size > 4096) return false;
    const saved = JSON.parse(await marker.text());
    if (saved.version !== VERSION || saved.sha256 !== MODEL_SHA256 || saved.model?.size !== MODEL_BYTES) return false;
    const model = await fileInfo(dir, MODEL), config = await fileInfo(dir, `${MODEL}.json`);
    return model.size === saved.model.size && model.lastModified === saved.model.lastModified
      && config.size > 0 && config.size === saved.config?.size && config.lastModified === saved.config.lastModified;
  } catch { return false; }
}

async function writeFile(dir, name, bytes) {
  const file = await dir.getFileHandle(name, { create: true });
  const writable = await file.createWritable();
  try {
    await writable.write(bytes);
    await writable.close();
  } catch (error) {
    await writable.abort().catch(() => {});
    throw error;
  }
}

async function installModel(cache, dir) {
  const response = await fetch(`/voice/${VERSION}/manifest.json`);
  if (!response.ok) throw new Error(`manifest HTTP ${response.status}`);
  const manifest = await response.json();
  if (manifest.voice !== "en_US-amy-medium" || !Array.isArray(manifest.chunks)
    || manifest.chunks.length !== 4 || manifest.chunks.some((part, i) => part !== `model.part-0${i}`)) {
    throw new Error("Invalid voice manifest");
  }
  const model = new Uint8Array(MODEL_BYTES);
  let offset = 0;
  const urls = manifest.chunks.map(part => `/voice/${VERSION}/${part}`);
  for (const url of urls) {
    let response = await cache.match(url);
    let save;
    if (!response) {
      response = await fetch(url);
      if (!response.ok) throw new Error(`voice HTTP ${response.status}`);
      // Consume both tee branches together, instead of retaining an entire unread chunk.
      save = cache.put(url, response.clone());
      save.catch(() => {});
    }
    const reader = response.body.getReader();
    let exceededSize = false;
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        if (offset + value.byteLength > model.length) { exceededSize = true; break; }
        model.set(value, offset);
        offset += value.byteLength;
        status("Amy — preparing", offset / model.length);
      }
    } finally {
      await reader.cancel().catch(() => {});
      reader.releaseLock();
      await save;
    }
    if (exceededSize) {
      await Promise.all(urls.map(url => cache.delete(url)));
      throw new Error("Voice model exceeds expected size");
    }
  }
  if (offset !== MODEL_BYTES || await sha256(model) !== MODEL_SHA256) {
    await Promise.all(urls.map(url => cache.delete(url)));
    throw new Error("voice checksum or size mismatch");
  }
  const configResponse = await cache.match(`/voice/${VERSION}/${MODEL}.json`);
  const config = await configResponse.arrayBuffer();
  if (!config.byteLength || config.byteLength > 2 * 1024 * 1024) throw new Error("Invalid voice configuration size");
  await dir.removeEntry(VERIFIED).catch(() => {});
  await writeFile(dir, MODEL, model);
  await writeFile(dir, `${MODEL}.json`, config);
  await writeFile(dir, VERIFIED, JSON.stringify({
    version: VERSION, sha256: MODEL_SHA256,
    model: await fileInfo(dir, MODEL), config: await fileInfo(dir, `${MODEL}.json`),
  }));
}

function startWorker() {
  const candidate = new Worker(WORKER, { type: "module" });
  worker = candidate;
  return new Promise((resolve, reject) => {
    let settled = false;
    const fail = message => {
      if (worker !== candidate) return;
      worker = null;
      candidate.terminate();
      clearTimeout(timer);
      nextJob++;
      clearAudio();
      status(`Amy unavailable (${message})`);
      if (!settled) { settled = true; reject(new Error(message)); }
    };
    const timer = setTimeout(() => fail("voice initialization timed out; try again"), INIT_TIMEOUT_MS);
    candidate.onerror = error => {
      error.preventDefault?.();
      fail(error.message || "voice worker failed");
    };
    candidate.onmessageerror = () => fail("voice worker message could not be read");
    candidate.onmessage = ({ data }) => {
      if (worker !== candidate) return;
      if (data.type === "ready" && !settled) {
        settled = true;
        clearTimeout(timer);
        status("Amy — ready");
        resolve();
      } else if (data.type === "error" && (!settled || data.id === nextJob)) {
        fail(data.message || "voice initialization failed");
      } else if (data.type === "audio" && settled && data.id === nextJob) {
        clearAudio();
        audioUrl = URL.createObjectURL(new Blob([data.bytes], { type: "audio/wav" }));
        audio = new Audio(audioUrl);
        audio.volume = Math.max(0, Math.min(1, data.volume ?? 1));
        audio.onended = clearAudio;
        audio.onerror = clearAudio;
        globalThis.__hookechoAmySpeaking = true;
        audio.play().catch(error => fail(error.message));
      }
    };
    try { candidate.postMessage({ type: "prepare" }); }
    catch (error) { fail(error.message); }
  });
}

/** Called only when speech is enabled. Reuse verified OPFS bytes on subsequent visits. */
export function prepareAmy() {
  if (globalThis.__hookechoAmyPreparing) return globalThis.__hookechoAmyPreparing;
  if (worker && globalThis.__hookechoAmyStatus === "Amy — ready") return Promise.resolve(true);
  globalThis.__hookechoAmyPreparing = (async () => {
    status("Amy — preparing", 0);
    try {
      const cache = await caches.open(CACHE);
      for (const url of [
        WORKER,
        "/voice/runtime/piper-tts-web.js?module=1",
        "/voice/runtime/piper-o91UDS6e.js",
        "/voice/runtime/ort.wasm.min.js?module=1",
        "/voice/runtime/ort-wasm.wasm",
        "/voice/runtime/ort-wasm-simd.wasm",
        "/voice/runtime/ort-wasm-simd-threaded.wasm",
        "/voice/runtime/piper_phonemize.data",
        "/voice/runtime/piper_phonemize.wasm",
        `/voice/${VERSION}/${MODEL}.json`,
      ]) {
        if (!await cache.match(url)) await cache.add(url);
      }
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("piper", { create: true });
      if (!await verifiedModel(dir)) await installModel(cache, dir);
      status("Amy — starting voice");
      await startWorker();
      return true;
    } catch (error) {
      status(`Amy unavailable (${error.message || error})`);
      return false;
    }
  })().finally(() => { globalThis.__hookechoAmyPreparing = null; });
  return globalThis.__hookechoAmyPreparing;
}
