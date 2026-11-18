// `node --test web/decode-bridge.test.mjs`
//
// The bridge's happy path is covered by every page load; what is not is what happens when the
// worker dies, which is the failure that made a browser tab unusable for the rest of a session.
// A stub worker lets us ask for a trap on demand.
import test from "node:test";
import assert from "node:assert/strict";
import { makeBridge } from "./decode-bridge.js";

test("radar goes ahead of queued vector work", async () => {
  const { bridge, spawned } = harness([[]]);
  await Promise.all([
    bridge(new Uint8Array([1]), "vector"),
    bridge(new Uint8Array([2]), "vector"),
    bridge(new Uint8Array([3]), "assemble"),
  ]);
  assert.deepEqual(spawned[0].jobs.map(job => job.op), ["vector", "assemble", "vector"]);
});

/// A worker that answers every job the way the script says, in order, and remembers whether it
/// was terminated. `reply` is `{ ok }`, `{ err }` or `{ err, fatal }`.
function stubWorker(replies) {
  const w = {
    terminated: false,
    jobs: [],
    transfers: [],
    terminate() {
      this.terminated = true;
    },
    postMessage(msg, transfer) {
      this.jobs.push(msg);
      this.transfers.push(transfer);
      const reply = replies.shift() ?? { ok: new Uint8Array([0]).buffer };
      // Asynchronous like the real thing: a synchronous answer would hide ordering bugs.
      queueMicrotask(() => this.onmessage({ data: { id: msg.id, ...reply } }));
    },
  };
  return w;
}

function harness(scripts) {
  const spawned = [];
  const bridge = makeBridge({
    spawnWorker: () => {
      const w = stubWorker(scripts.shift() ?? []);
      spawned.push(w);
      return w;
    },
    log: () => {},
  });
  return { bridge, spawned };
}

test("a job carries its op and transfers its buffer", async () => {
  const { bridge, spawned } = harness([[{ ok: new Uint8Array([7, 8]).buffer }]]);
  const out = await bridge(new Uint8Array([1, 2, 3]), "assemble");
  assert.deepEqual([...out], [7, 8]);
  assert.equal(spawned[0].jobs[0].op, "assemble");
  assert.equal(spawned[0].transfers[0].length, 1);
});

test("an ordinary failure rejects that job and keeps the worker", async () => {
  const { bridge, spawned } = harness([[{ err: "bad volume" }, { ok: new Uint8Array([9]).buffer }]]);
  await assert.rejects(() => bridge(new Uint8Array([1]), "decode"), /bad volume/);
  const out = await bridge(new Uint8Array([1]), "decode");
  assert.deepEqual([...out], [9]);
  assert.equal(spawned.length, 1, "no respawn for a bad volume");
  assert.equal(spawned[0].terminated, false);
});

test("a trap replaces the worker, and the next job uses the new one", async () => {
  const { bridge, spawned } = harness([
    [{ err: "unreachable", fatal: true }],
    [{ ok: new Uint8Array([5]).buffer }],
  ]);
  await assert.rejects(() => bridge(new Uint8Array([1]), "decode"), /unreachable/);
  assert.equal(spawned.length, 2, "a trap respawns");
  assert.equal(spawned[0].terminated, true);
  const out = await bridge(new Uint8Array([1]), "decode");
  assert.deepEqual([...out], [5]);
  assert.equal(spawned[1].jobs.length, 1);
});

test("a trap retains queued peers for the replacement worker in priority order", async () => {
  const { bridge, spawned } = harness([
    [{ err: "unreachable", fatal: true }],
    [{ ok: new Uint8Array([5]).buffer }, { ok: new Uint8Array([6]).buffer }],
  ]);
  const broken = assert.rejects(bridge(new Uint8Array([1]), "mrms"), /unreachable/);
  const vector = bridge(new Uint8Array([2]), "vector");
  const radar = bridge(new Uint8Array([3]), "assemble");
  const [, map, scan] = await Promise.all([broken, vector, radar]);
  assert.deepEqual([...scan], [5]);
  assert.deepEqual([...map], [6]);
  assert.deepEqual(spawned[1].jobs.map(job => job.op), ["assemble", "vector"]);
  assert.deepEqual([...new Uint8Array(spawned[1].jobs[0].bytes)], [3]);
});

test("a worker error only falls back the transferred job, not queued peers", async () => {
  const first = { jobs: [], postMessage(job) { this.jobs.push(job); }, terminate() {} };
  const second = stubWorker([{ ok: new Uint8Array([7]).buffer }]);
  const workers = [first, second];
  const bridge = makeBridge({ spawnWorker: () => workers.shift(), log: () => {} });
  const failed = assert.rejects(bridge(new Uint8Array([1]), "model"), /worker unavailable/);
  const waiting = bridge(new Uint8Array([2]), "bin");
  first.onerror({ message: "crashed" });
  assert.deepEqual([...await waiting], [7]);
  await failed;
});

test("a failed respawn or exhausted budget settles queued peers instead of stranding them", async () => {
  for (const maxRespawns of [0, 1]) {
    const first = { postMessage() {}, terminate() {} };
    let calls = 0;
    const bridge = makeBridge({
      spawnWorker: () => { if (calls++) throw new Error("boot unavailable"); return first; },
      maxRespawns, log: () => {},
    });
    const running = assert.rejects(bridge(new Uint8Array([1]), "model"), /worker unavailable/);
    const waiting = assert.rejects(bridge(new Uint8Array([2]), "bin"), /worker unavailable/);
    first.onerror({ message: "crashed" });
    await Promise.all([running, waiting]);
  }
});

test("repeated traps stop respawning, and then jobs fall back inline", async () => {
  const trap = [{ err: "unreachable", fatal: true }];
  const { bridge, spawned } = harness([trap, [...trap], [...trap], [...trap]]);
  for (let i = 0; i < 4; i++) {
    await assert.rejects(() => bridge(new Uint8Array([1]), "decode"));
  }
  assert.equal(spawned.length, 4, "three respawns after the original, then no more");
  // `"worker unavailable"` is the string wasm_worker.rs reads as "decode this inline".
  await assert.rejects(() => bridge(new Uint8Array([1]), "decode"), /worker unavailable/);
});

test("no workers at all means every job decodes inline", async () => {
  const bridge = makeBridge({
    spawnWorker: () => {
      throw new Error("no module workers here");
    },
    log: () => {},
  });
  await assert.rejects(() => bridge(new Uint8Array([1]), "decode"), /worker unavailable/);
});

test("vector tile jobs preserve their operation and transferred payload", async () => {
  const { bridge, spawned } = harness([[{ ok: new Uint8Array([7]).buffer }]]);
  assert.deepEqual([...await bridge(new Uint8Array([3, 4]), "vector")], [7]);
  assert.equal(spawned[0].jobs[0].op, "vector");
  assert.deepEqual([...new Uint8Array(spawned[0].jobs[0].bytes)], [3, 4]);
});

test("visible display preparation runs before queued downloads and map refinement", async () => {
  const worker = { jobs: [], postMessage(job) { this.jobs.push(job); }, terminate() {} };
  const bridge = makeBridge({ spawnWorker: () => worker, log: () => {} });
  const running = bridge(new Uint8Array([1]), "mrms");
  const history = bridge(new Uint8Array([2]), "decode");
  const vector = bridge(new Uint8Array([3]), "vector");
  const display = bridge(new Uint8Array([4]), "bin");
  const finish = () => {
    const job = worker.jobs.at(-1);
    worker.onmessage({ data: { id: job.id, ok: new Uint8Array([9]).buffer } });
  };
  finish();
  assert.equal(worker.jobs[1].op, "bin");
  finish();
  assert.equal(worker.jobs[2].op, "decode");
  finish();
  assert.equal(worker.jobs[3].op, "vector");
  finish();
  await Promise.all([running, history, vector, display]);
});

test("abandoning queued work releases it and preserves the active job and peers", async () => {
  const worker = { jobs: [], postMessage(job) { this.jobs.push(job); }, terminate() {} };
  const bridge = makeBridge({ spawnWorker: () => worker, log: () => {} });
  const running = bridge(new Uint8Array([1]), "decode");
  const stale = bridge(new Uint8Array([2]), "model");
  const current = bridge(new Uint8Array([3]), "vector");
  const cancelled = assert.rejects(stale, /decode cancelled/);
  stale.cancel();
  stale.cancel();
  worker.onmessage({ data: { id: worker.jobs[0].id, ok: new Uint8Array([4]).buffer } });
  assert.deepEqual(worker.jobs.map(job => job.op), ["decode", "vector"]);
  worker.onmessage({ data: { id: worker.jobs[1].id, ok: new Uint8Array([5]).buffer } });
  await Promise.all([running, current, cancelled]);
});

test("abandoning an active job cannot strand the queue or dispatch concurrent decodes", async () => {
  const worker = { jobs: [], postMessage(job) { this.jobs.push(job); }, terminate() {} };
  const bridge = makeBridge({ spawnWorker: () => worker, log: () => {} });
  const stale = bridge(new Uint8Array([1]), "decode");
  const current = bridge(new Uint8Array([2]), "model");
  const cancelled = assert.rejects(stale, /decode cancelled/);
  stale.cancel();
  assert.equal(worker.jobs.length, 1);
  worker.onmessage({ data: { id: worker.jobs[0].id, ok: new Uint8Array([4]).buffer } });
  assert.equal(worker.jobs.length, 2);
  worker.onmessage({ data: { id: worker.jobs[1].id, ok: new Uint8Array([5]).buffer } });
  await Promise.all([current, cancelled]);
});

test("worker timing separates queued contention, round trip and decode and retains exact context", async () => {
  let now = 10;
  const worker = { jobs: [], postMessage(job) { this.jobs.push(job); }, terminate() {} };
  const bridge = makeBridge({ spawnWorker: () => worker, clock: { now: () => now, timeOrigin: 1000 } });
  const model = bridge(new Uint8Array([1]), "model");
  now = 15;
  const context = { site: "KTLX", volume: "20261118_120000", sweep: 2, pane: 0 };
  const radar = bridge(new Uint8Array([2]), "assemble", context);
  now = 70;
  worker.onmessage({ data: { id: worker.jobs[0].id, ok: new ArrayBuffer(0) } });
  now = 85;
  worker.onmessage({ data: { id: worker.jobs[1].id, ok: new ArrayBuffer(0), timing: {
    workerStartAtUtcMs: 1072, workerEndAtUtcMs: 1082, decodeMs: 10,
  } } });
  await Promise.all([model, radar]);
  assert.deepEqual(bridge.timings[1], {
    id: 2, op: "assemble", context, status: "ok", queuedAtUtcMs: 1015,
    dispatchedAtUtcMs: 1070, receivedAtUtcMs: 1085, queueMs: 55, roundTripMs: 15,
    workerStartAtUtcMs: 1072, workerEndAtUtcMs: 1082, decodeMs: 10,
  });
  assert.equal(bridge.timings[0].decodeMs, null, "old workers remain compatible without invented timing");
});

test("worker timing storage is bounded and cancelled queue entries never claim decode time", async () => {
  const { bridge } = harness([[]]);
  for (let i = 0; i < 130; i++) await bridge(new Uint8Array([1]), "decode");
  assert.equal(bridge.timings.length, 128);
  assert.equal(bridge.timings[0].id, 3);
  const running = bridge(new Uint8Array([2]), "model");
  const queued = bridge(new Uint8Array([3]), "bin");
  const cancelled = assert.rejects(queued, /decode cancelled/);
  queued.cancel();
  const trace = bridge.timings.at(-1);
  assert.equal(trace.status, "cancelled");
  assert.equal(trace.dispatchedAtUtcMs, null);
  assert.equal(trace.decodeMs, null);
  await Promise.all([running, cancelled]);
});
