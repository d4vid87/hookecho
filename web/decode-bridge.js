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
export function makeBridge({ spawnWorker, maxRespawns = 3, log = console.warn, clock = performance }) {
  const pending = new Map();
  const timings = [];
  let worker = null;
  let nextId = 1;
  const queued = [];
  let active = null;
  const finish = (id, status, workerTiming) => {
    const entry = pending.get(id);
    if (!entry) return null;
    pending.delete(id);
    const [, , trace] = entry;
    const received = clock.now();
    timings.push({
      id, op: trace.op, context: trace.context, status,
      queuedAtUtcMs: clock.timeOrigin + trace.queued,
      dispatchedAtUtcMs: trace.dispatched === null ? null : clock.timeOrigin + trace.dispatched,
      receivedAtUtcMs: clock.timeOrigin + received,
      queueMs: trace.dispatched === null ? null : trace.dispatched - trace.queued,
      roundTripMs: trace.dispatched === null ? null : received - trace.dispatched,
      workerStartAtUtcMs: workerTiming?.workerStartAtUtcMs ?? null,
      workerEndAtUtcMs: workerTiming?.workerEndAtUtcMs ?? null,
      decodeMs: workerTiming?.decodeMs ?? null,
    });
    if (timings.length > 128) timings.shift();
    return entry;
  };
  const dispatch = () => {
    if (active !== null || !worker || !queued.length) return;
    // Keep radar work ahead of queued map refinement. Never interrupt an executing decode.
    const urgent = queued.findIndex(job => job.op === "bin" || job.op === "assemble");
    const radar = queued.findIndex(job => job.op !== "vector" && job.op !== "import");
    const job = queued.splice(urgent >= 0 ? urgent : radar < 0 ? 0 : radar, 1)[0];
    active = job.id;
    pending.get(job.id)[2].dispatched = clock.now();
    worker.postMessage(job, [job.bytes]);
  };
  // A trap that repeats is a bug in the decode, not bad luck, and respawning forever would turn
  // it into a loop.
  let respawns = 0;

  // Replace the worker. A wasm trap poisons its heap, so the instance has to go — but the *next*
  // volume has a fresh heap and every reason to succeed, and a session that decodes inline from
  // then on is a session of multi-second freezes. Rebuild, up to a limit.
  const retire = (why) => {
    if (worker) worker.terminate();
    worker = null;
    // Only the active buffer was transferred. Queued peers can use the replacement heap.
    const entry = finish(active, "worker unavailable");
    if (entry) {
      entry[1](new Error("worker unavailable"));
    }
    active = null;
    if (++respawns <= maxRespawns) {
      log(`decode worker restarting (${respawns}/${maxRespawns}):`, why);
      attach();
    } else {
      log("decode worker retired:", why);
    }
    if (worker) dispatch();
    else {
      // `wasm_worker.rs` recognizes this exact error and decodes inline as a last resort.
      for (const [id] of pending) finish(id, "worker unavailable")[1](new Error("worker unavailable"));
      queued.length = 0;
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
    const attached = worker;
    worker.onerror = (e) => {
      if (worker === attached) retire((e && e.message) || "worker error");
    };
    worker.onmessage = (e) => {
      if (worker !== attached) return;
      const { id, ok, err, fatal, timing } = e.data;
      if (id === active) active = null;
      const entry = finish(id, err ? "error" : "ok", timing);
      if (entry) {
        const [resolve, reject] = entry;
        if (err) reject(new Error(err));
        else resolve(new Uint8Array(ok));
      }
      // A trap poisons the heap this instance decodes in: everything after it would fail too. The
      // job that hit it was already answered above; queued peers keep their untransferred bytes.
      if (fatal) retire("wasm trap");
      else dispatch();
    };
  };

  attach();

  // What wasm_worker.rs looks up as `globalThis.__decodeVolume`. `op` names the export the worker
  // should call. Only "worker unavailable" requests inline fallback; bad input stays an error.
  const bridge = (bytes, op, context = null) => {
    if (!worker) return Promise.reject(new Error("worker unavailable"));
    const id = nextId++;
    const promise = new Promise((resolve, reject) => {
      pending.set(id, [resolve, reject, { op, context, queued: clock.now(), dispatched: null }]);
      // Transfer, not copy: the volume is tens of MB and this side is finished with it. Rust
      // hands us a fresh JS-heap array for exactly this reason.
      queued.push({ id, op, bytes: bytes.buffer });
      dispatch();
    });
    // Rust drops abandoned futures; remove their queued bytes before a stale decode starts.
    promise.cancel = () => {
      const entry = finish(id, "cancelled");
      if (!entry) return;
      const index = queued.findIndex(job => job.id === id);
      if (index >= 0) queued.splice(index, 1);
      entry[1](new Error("decode cancelled"));
      // An executing synchronous decode finishes once; it must still release the worker slot.
    };
    return promise;
  };
  // This bounded job trace measures processing only, never observation age or presentation.
  bridge.timings = timings;
  return bridge;
}
