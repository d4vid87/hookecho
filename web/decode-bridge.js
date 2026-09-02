// The page's half of the decode-worker bridge: job ids, transfers, and what to do when the
// worker dies.
//
// It lives in its own module rather than inline in index.html for one reason: the interesting
// behaviour is the failure path, and a trap is not something a browser test can ask for on
// demand. `spawnWorker` is injected, so `decode-bridge.test.mjs` can hand it a stub that reports
// a trap and check that the next job still reaches a worker.
//
// See web/decode-worker.js for the protocol and crates/wxdata/src/wasm_worker.rs for the caller.

/// Build the bridge. `spawnWorker()` returns a fresh worker (already sent its boot message);
/// returning null means there are no workers here at all and every job should decode inline.
export function makeBridge({ spawnWorker, maxRespawns = 3, log = console.warn }) {
  const pending = new Map();
  let worker = null;
  let nextId = 1;
  // A trap that repeats is a bug in the decode, not bad luck, and respawning forever would turn
  // it into a loop.
  let respawns = 0;

  // Replace the worker. A wasm trap poisons its heap, so the instance has to go — but the *next*
  // volume has a fresh heap and every reason to succeed, and a session that decodes inline from
  // then on is a session of multi-second freezes. Rebuild, up to a limit.
  const retire = (why) => {
    if (worker) worker.terminate();
    worker = null;
    // `"worker unavailable"` is the string wasm_worker.rs reads to tell "use the fallback this
    // once" apart from "this volume is bad".
    for (const [, [, reject]] of pending) reject(new Error("worker unavailable"));
    pending.clear();
    if (++respawns <= maxRespawns) {
      log(`decode worker restarting (${respawns}/${maxRespawns}):`, why);
      attach();
    } else {
      log("decode worker retired:", why);
    }
  };

  const attach = () => {
    try {
      worker = spawnWorker();
    } catch (e) {
      // No module workers (or a `file://` page). Not fatal — the app decodes inline.
      log("no decode worker:", e);
      worker = null;
      return;
    }
    if (!worker) return;
    worker.onerror = (e) => retire((e && e.message) || "worker error");
    worker.onmessage = (e) => {
      const { id, ok, err, fatal } = e.data;
      const entry = pending.get(id);
      if (entry) {
        pending.delete(id);
        const [resolve, reject] = entry;
        if (err) reject(new Error(err));
        else resolve(new Uint8Array(ok));
      }
      // A trap poisons the heap this instance decodes in: everything after it would fail too. The
      // job that hit it was already answered above, so this only costs the jobs still queued.
      if (fatal) retire("wasm trap");
    };
  };

  attach();

  // What wasm_worker.rs looks up as `globalThis.__decodeVolume`. `op` names the export the worker
  // should call. A rejection means "do this one inline", never "give up on the worker".
  return (bytes, op) => {
    if (!worker) return Promise.reject(new Error("worker unavailable"));
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, [resolve, reject]);
      // Transfer, not copy: the volume is tens of MB and this side is finished with it. Rust
      // hands us a fresh JS-heap array for exactly this reason.
      worker.postMessage({ id, op, bytes: bytes.buffer }, [bytes.buffer]);
    });
  };
}
