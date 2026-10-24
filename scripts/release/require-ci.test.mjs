import {test} from 'node:test';
import assert from 'node:assert/strict';
import {ciState} from './require-ci.mjs';

const sha = 'a'.repeat(40);
const run = {head_sha: sha, head_branch: 'main', event: 'push', run_number: 1, run_attempt: 1, status: 'completed', conclusion: 'success'};
test('publishing requires successful push CI for exactly this main commit', () => {
  assert.equal(ciState([run], sha), 'success');
  for (const changed of [{head_sha: 'b'.repeat(40)}, {event: 'pull_request'}, {head_branch: 'other'}]) {
    assert.equal(ciState([{...run, ...changed}], sha), 'waiting');
  }
  for (const conclusion of ['failure', 'cancelled', 'timed_out', 'skipped', null]) {
    assert.equal(ciState([{...run, conclusion}], sha), 'failure');
  }
  assert.equal(ciState([{...run, status: 'in_progress'}], sha), 'waiting');
  assert.equal(ciState([], sha), 'waiting');
});
test('a pending or failing rerun cannot inherit an older success', () => {
  assert.equal(ciState([run, {...run, run_attempt: 2, status: 'in_progress'}], sha), 'waiting');
  assert.equal(ciState([run, {...run, run_number: 2, conclusion: 'failure'}], sha), 'failure');
});
