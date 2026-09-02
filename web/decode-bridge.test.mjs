// `node --test web/decode-bridge.test.mjs`
//
// The bridge's happy path is covered by every page load; what is not is what happens when the
// worker dies, which is the failure that made a browser tab unusable for the rest of a session.
// A stub worker lets us ask for a trap on demand.
import test from "node:test";
import assert from "node:assert/strict";
import { makeBridge } from "./decode-bridge.js";

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
