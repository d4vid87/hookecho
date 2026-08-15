// Level 2 decode, off the thread that draws the map.
//
// A volume is a gzip wrapper around ~130 bzip2 records — seconds of solid CPU, and on the main
// thread that is seconds of frozen map. This worker runs a second instance of the *same* wasm
// module the page already compiled (the page transfers the `WebAssembly.Module` itself, which is
// structured-cloneable, so there is no second download and no second compile) and calls exactly
// one export: `decode_archive2`. The app never boots here.
//
// The heap is the point as much as the thread: decoding peaks at well over 100 MB of scratch, and
// here that lives in a throwaway wasm memory instead of the one holding every texture and tile.
//
// Protocol, both directions over postMessage:
//   in   { module, glue }        boot: compiled module + the URL of the wasm-bindgen glue
//   in   { id, bytes }           decode this volume (bytes' buffer is transferred in)
//   out  { id, ok: ArrayBuffer } postcard-encoded Scan (transferred out)
//   out  { id, err: string }     this volume did not decode

let ready = null;

async function boot(module, glue) {
  // Dynamic import so this file names no content-hashed asset: the page, which does know the
  // hashed name, passes it in. That keeps decode-worker.js itself unhashed and cache-bustable.
  const wasm = await import(glue);
  await wasm.default({ module_or_path: module });
  return wasm;
}

self.onmessage = async (e) => {
  const msg = e.data;

  if (msg.module) {
    ready = boot(msg.module, msg.glue);
    // A boot failure must not become an unhandled rejection that kills the worker before the
    // page can hear about it; the first decode reports it instead.
    ready.catch(() => {});
    return;
  }

  const { id, bytes } = msg;
  try {
    const wasm = await ready;
    const out = wasm.decode_archive2(new Uint8Array(bytes));
    // Transfer rather than copy: a decoded volume is tens of MB and the worker is done with it.
    self.postMessage({ id, ok: out.buffer }, [out.buffer]);
  } catch (err) {
    self.postMessage({ id, err: String((err && err.message) || err) });
  }
};
