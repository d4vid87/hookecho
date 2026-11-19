import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';

const source = await fs.readFile(new URL('./decode-worker.js', import.meta.url), 'utf8');

test('document imports dispatch to the import export and transfer prepared bytes', async () => {
  const calls = [], replies = [];
  const context = vm.createContext({
    self: {postMessage: (...args) => replies.push(args)}, performance, WebAssembly, Uint8Array,
    wasm: {prepare_import_document(bytes) { calls.push([...bytes]); return new Uint8Array([7, 9]); },
      decode_archive2() { throw new Error('import went to radar decoder'); }},
  });
  vm.runInContext(`${source}\nready = Promise.resolve(wasm);`, context);
  await context.self.onmessage({data: {id: 12, op: 'import', bytes: new Uint8Array([1, 2, 3]).buffer}});
  assert.deepEqual(calls, [[1, 2, 3]]);
  assert.equal(replies[0][0].id, 12);
  assert.deepEqual([...new Uint8Array(replies[0][0].ok)], [7, 9]);
  assert.equal(replies[0][1][0], replies[0][0].ok);
});

test('invalid imported documents fail only their job and allow the next import', async () => {
  const replies = [];
  let bad = true;
  const context = vm.createContext({
    self: {postMessage: message => replies.push(message)}, performance, WebAssembly, Uint8Array,
    wasm: {prepare_import_document() {
      if (bad) { bad = false; throw new Error('GIS exceeds 500,000 vertices'); }
      return new Uint8Array([8]);
    }},
  });
  vm.runInContext(`${source}\nready = Promise.resolve(wasm);`, context);
  await context.self.onmessage({data: {id: 1, op: 'import', bytes: new ArrayBuffer(0)}});
  await context.self.onmessage({data: {id: 2, op: 'import', bytes: new ArrayBuffer(0)}});
  assert.equal(replies[0].err, 'GIS exceeds 500,000 vertices');
  assert.equal(replies[0].fatal, false);
  assert.deepEqual([...new Uint8Array(replies[1].ok)], [8]);
});
