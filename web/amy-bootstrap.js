const VERSION = "amy-medium-39ab474b";
const CACHE = `voice-${VERSION}`;
const MODEL_SHA256 = "b3a6e47b57b8c7fbe6a0ce2518161a50f59a9cdd8a50835c02cb02bdd6206c18";

function status(text, progress = null) {
  globalThis.__hookechoAmyStatus = progress == null ? text : `${text} ${Math.round(progress * 100)}%`;
  document.documentElement.dataset.amyStatus = globalThis.__hookechoAmyStatus;
}

let worker;
let audio;
let nextJob = 0;

function fallback(text, volume) {
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.volume = volume;
  speechSynthesis.speak(utterance);
}

globalThis.__hookechoAmySpeak = (text, volume) => {
  if (!worker || globalThis.__hookechoAmyStatus !== "Amy — ready") {
    fallback(text, volume);
    return;
  }
  globalThis.__hookechoAmySpeaking = true;
  worker.postMessage({ type: "speak", id: ++nextJob, text });
};
globalThis.__hookechoAmyStop = () => {
  nextJob++;
  globalThis.__hookechoAmySpeaking = false;
  if (audio) { audio.pause(); audio.removeAttribute("src"); }
  speechSynthesis.cancel();
};

async function sha256(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Download after map startup, cache each <=20 MiB part, then verify the reconstructed model. */
export async function prepareAmy() {
  if (globalThis.__hookechoAmyPreparing) return globalThis.__hookechoAmyPreparing;
  globalThis.__hookechoAmyPreparing = (async () => {
    status("Amy — preparing", 0);
    try {
      const manifest = await fetch(`/voice/${VERSION}/manifest.json`).then((r) => {
        if (!r.ok) throw new Error(`manifest HTTP ${r.status}`);
        return r.json();
      });
      const cache = await caches.open(CACHE);
      await cache.addAll([
        "/amy-worker.js",
        "/voice/runtime/piper-tts-web.js",
        "/voice/runtime/piper-o91UDS6e.js",
        "/voice/runtime/ort.wasm.min.js",
        "/voice/runtime/ort-wasm.wasm",
        "/voice/runtime/ort-wasm-simd.wasm",
        "/voice/runtime/ort-wasm-simd-threaded.wasm",
        "/voice/runtime/piper_phonemize.data",
        "/voice/runtime/piper_phonemize.wasm",
        `/voice/${VERSION}/en_US-amy-medium.onnx.json`,
      ]);
      const chunks = [];
      for (let i = 0; i < manifest.chunks.length; i++) {
        const url = `/voice/${VERSION}/${manifest.chunks[i]}`;
        let response = await cache.match(url);
        if (!response) {
          response = await fetch(url);
          if (!response.ok) throw new Error(`voice HTTP ${response.status}`);
          await cache.put(url, response.clone());
        }
        chunks.push(new Uint8Array(await response.arrayBuffer()));
        status("Amy — preparing", (i + 1) / manifest.chunks.length);
      }
      const model = new Uint8Array(chunks.reduce((n, part) => n + part.length, 0));
      let offset = 0;
      for (const part of chunks) { model.set(part, offset); offset += part.length; }
      if (await sha256(model) !== MODEL_SHA256) {
        await caches.delete(CACHE);
        throw new Error("voice checksum mismatch");
      }
      globalThis.__hookechoAmyModel = model.buffer;
      const root = await navigator.storage.getDirectory();
      const dir = await root.getDirectoryHandle("piper", { create: true });
      for (const [name, bytes] of [
        ["en_US-amy-medium.onnx", model],
        ["en_US-amy-medium.onnx.json", new Uint8Array(await cache.match(`/voice/${VERSION}/en_US-amy-medium.onnx.json`).then((r) => r.arrayBuffer()))],
      ]) {
        const file = await dir.getFileHandle(name, { create: true });
        const writable = await file.createWritable();
        await writable.write(bytes);
        await writable.close();
      }
      worker = new Worker("/amy-worker.js", { type: "module" });
      worker.onmessage = ({ data }) => {
        if (data.type === "ready") status("Amy — ready");
        else if (data.type === "audio" && data.id === nextJob) {
          audio = new Audio(URL.createObjectURL(new Blob([data.bytes], { type: "audio/wav" })));
          audio.onended = () => { globalThis.__hookechoAmySpeaking = false; };
          audio.play().catch((error) => status(`Device voice — fallback (${error.message})`));
        } else if (data.type === "error" && data.id === nextJob) {
          globalThis.__hookechoAmySpeaking = false;
          status(`Device voice — fallback (${data.message})`);
        }
      };
      worker.postMessage({ type: "prepare" });
    } catch (error) {
      status(`Device voice — fallback (${error.message || error})`);
    }
  })();
  return globalThis.__hookechoAmyPreparing;
}
