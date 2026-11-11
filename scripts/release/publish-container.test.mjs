import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

test('container latest follows current main or stable tags, never stale main or prerelease tags', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'hookecho-container-'));
  try {
    const log = path.join(root, 'commands.jsonl');
    await fs.writeFile(path.join(root, 'docker'), `#!/usr/bin/env node
require('node:fs').appendFileSync(process.env.FAKE_LOG, JSON.stringify(process.argv.slice(2)) + '\\n');
`, {mode: 0o755});
    await fs.writeFile(path.join(root, 'gh'), `#!/usr/bin/env node
if (process.env.FAKE_HEAD_ERROR === '1') process.exit(1);
console.log(process.env.FAKE_MAIN_SHA);
`, {mode: 0o755});
    const sha = 'a'.repeat(40);
    const image = 'ghcr.io/owner/repo';
    const env = {...process.env, PATH: `${root}${path.delimiter}${process.env.PATH}`, IMAGE: image, GITHUB_SHA: sha, GITHUB_REPOSITORY: 'owner/repo', FAKE_LOG: log, FAKE_MAIN_SHA: sha};
    const script = fileURLToPath(new URL('./publish-container.sh', import.meta.url));
    for (const [ref, head, latest] of [
      ['refs/heads/main', sha, true],
      ['refs/heads/main', 'b'.repeat(40), false],
      ['refs/tags/v0.12.0', sha, true],
      ['refs/tags/v0.13.0-beta.1', sha, false],
    ]) {
      await fs.writeFile(log, '');
      const result = spawnSync('bash', [script], {env: {...env, GITHUB_REF: ref, FAKE_MAIN_SHA: head}, encoding: 'utf8'});
      assert.equal(result.status, 0, result.stderr);
      const commands = (await fs.readFile(log, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
      assert.deepEqual(commands, [
        ['push', `${image}:sha-${sha}`],
        ...(ref.startsWith('refs/tags/') ? [['push', `${image}:${ref.slice('refs/tags/v'.length)}`]] : []),
        ...(latest ? [['tag', `${image}:sha-${sha}`, `${image}:latest`], ['push', `${image}:latest`]] : []),
      ]);
    }
    await fs.writeFile(log, '');
    const unavailable = spawnSync('bash', [script], {env: {...env, GITHUB_REF: 'refs/heads/main', FAKE_HEAD_ERROR: '1'}, encoding: 'utf8'});
    assert.notEqual(unavailable.status, 0);
    assert(!(await fs.readFile(log, 'utf8')).includes(':latest'));
  } finally { await fs.rm(root, {recursive: true, force: true}); }
});
