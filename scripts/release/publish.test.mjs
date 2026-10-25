import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {releasePlan, collectReleaseFiles, requiredFiles} from './publish.mjs';

test('previews have immutable commit tags and never become the stable release', () => {
  const sha = 'a'.repeat(40);
  assert.deepEqual(releasePlan('refs/heads/main', sha), {tag: `build-${sha}`, title: `Preview ${sha.slice(0, 12)}`, prerelease: true});
  assert.equal(releasePlan('refs/tags/v1.2.3', sha).prerelease, false);
  assert.equal(releasePlan('refs/tags/v1.2.3-beta.1', sha).prerelease, true);
  assert.throws(() => releasePlan('refs/heads/other', sha));
  assert.throws(() => releasePlan('refs/heads/main', 'short'));
});
test('publishing refuses incomplete, empty, or duplicate platform packages', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'hookecho-release-'));
  try {
    await assert.rejects(collectReleaseFiles(root), /Missing/);
    for (const name of requiredFiles) await fs.writeFile(path.join(root, name), 'package');
    assert.equal((await collectReleaseFiles(root)).size, requiredFiles.length);
    await fs.writeFile(path.join(root, requiredFiles[0]), '');
    await assert.rejects(collectReleaseFiles(root), /Empty/);
    await fs.writeFile(path.join(root, requiredFiles[0]), 'package');
    await fs.mkdir(path.join(root, 'duplicate'));
    await fs.writeFile(path.join(root, 'duplicate', requiredFiles[0]), 'package');
    await assert.rejects(collectReleaseFiles(root), /Duplicate/);
  } finally { await fs.rm(root, {recursive: true, force: true}); }
});

test('failed uploads remain drafts, retry finishes, and published packages are never overwritten', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'hookecho-publish-'));
  try {
    const artifacts = path.join(root, 'artifacts');
    const bin = path.join(root, 'bin');
    await fs.mkdir(artifacts); await fs.mkdir(bin);
    for (const name of requiredFiles) await fs.writeFile(path.join(artifacts, name), 'package');
    const state = path.join(root, 'state.json');
    const log = path.join(root, 'commands.jsonl');
    await fs.writeFile(path.join(bin, 'gh'), `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FAKE_LOG, JSON.stringify(args) + '\\n');
const state = process.env.FAKE_STATE;
if (args[0] === 'api') {
  if (args[1].endsWith('/tags/latest')) { console.error('HTTP 404'); process.exit(1); }
  if (!fs.existsSync(state)) { console.error('HTTP 404'); process.exit(1); }
  console.log(fs.readFileSync(state, 'utf8'));
} else if (args[1] === 'create') {
  fs.writeFileSync(state, JSON.stringify({draft:true,target_commitish:process.env.GITHUB_SHA,body:fs.readFileSync(args[args.indexOf('--notes-file')+1], 'utf8')}));
} else if (args[1] === 'upload') {
  if (!JSON.parse(fs.readFileSync(state)).draft) throw new Error('published upload');
  if (process.env.FAKE_FAIL === '1') process.exit(1);
} else if (args[1] === 'edit') {
  const release = JSON.parse(fs.readFileSync(state)); release.draft = false;
  fs.writeFileSync(state, JSON.stringify(release));
} else throw new Error('unexpected command');
`, {mode: 0o755});
    const env = {...process.env, PATH: `${bin}${path.delimiter}${process.env.PATH}`, GITHUB_REF: 'refs/heads/main', GITHUB_SHA: 'a'.repeat(40), GITHUB_REPOSITORY: 'owner/repo', FAKE_STATE: state, FAKE_LOG: log};
    const script = fileURLToPath(new URL('./publish.mjs', import.meta.url));
    const run = extra => spawnSync(process.execPath, [script, artifacts], {cwd: root, env: {...env, ...extra}, encoding: 'utf8'});
    assert.notEqual(run({FAKE_FAIL: '1'}).status, 0);
    assert.equal(JSON.parse(await fs.readFile(state)).draft, true);
    const retry = run({}); assert.equal(retry.status, 0, retry.stderr);
    assert.equal(JSON.parse(await fs.readFile(state)).draft, false);
    const before = await fs.readFile(log, 'utf8');
    assert.equal(run({}).status, 0);
    const added = (await fs.readFile(log, 'utf8')).slice(before.length);
    assert(!added.includes('upload'));
    assert.equal((await fs.readFile(path.join(root, 'dist/publish/SHA256SUMS'), 'utf8')).trim().split('\n').length, requiredFiles.length);
  } finally { await fs.rm(root, {recursive: true, force: true}); }
});
