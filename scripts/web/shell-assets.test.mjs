import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, mkdir, writeFile, readFile, rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {pageAssets, shellAssets, buildShell} from './shell-assets.mjs';

const html = `<link href="./dist/hookecho-12345678.js?boot=2">
<link href="./dist/hookecho_bg-abcdef01.wasm">
<script src="./dist/video-player-11111111.js"></script>
<script>import('./dist/decode-bridge-22222222.js'); new Worker(new URL('./dist/decode-worker-33333333.js?v=radar-fields', import.meta.url));
navigator.serviceWorker.register('/sw.js'); globalThis.__fontUrls={face:'/dist/font-Face-44444444.ttf'};</script>`;
const glue = `import { offlineRead } from './snippets/hookecho/inline0-1234abcd.js';
import * as import1 from "./snippets/hookecho/inline1-abcd5678.js"
import { other } from './snippets/hookecho/inline0-1234abcd.js';`;

test('the shell includes actual module imports, helper versions, and fonts exactly once', async () => {
  const assets = await shellAssets(html, async () => glue);
  for (const asset of ['/dist/snippets/hookecho/inline0-1234abcd.js', '/dist/snippets/hookecho/inline1-abcd5678.js',
    '/dist/decode-worker-33333333.js?v=radar-fields', '/dist/decode-bridge-22222222.js', '/dist/font-Face-44444444.ttf']) {
    assert.equal(assets.filter(value => value === asset).length, 1, asset);
  }
  assert(!assets.includes('/sw.js'));
  await assert.rejects(shellAssets(html, async () => "import { read } from './snippets/hookecho/inline0.js';"), /content-hashed/);
  assert.throws(() => pageAssets('<html>error page</html>'), /bundle references/);
});

test('generation fails for a missing dependency and versions HTML/helper-only changes', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'hookecho-shell-'));
  try {
    const assets = await shellAssets(html, async () => glue);
    for (const asset of assets) {
      const file = path.join(root, asset === '/' ? 'index.html' : asset.split('?')[0]);
      await mkdir(path.dirname(file), {recursive: true});
      await writeFile(file, asset === '/' ? html : asset.includes('hookecho-12345678.js') ? glue : 'asset');
    }
    await writeFile(path.join(root, 'sw.src.js'), 'const SHELL = __SHELL__; const CACHE = __VERSION__;');
    await buildShell(root);
    const first = await readFile(path.join(root, 'sw.js'), 'utf8');
    await writeFile(path.join(root, 'index.html'), html + '<!-- copy change -->');
    await buildShell(root);
    const second = await readFile(path.join(root, 'sw.js'), 'utf8');
    assert.notEqual(first, second);
    await writeFile(path.join(root, 'dist/video-player-11111111.js'), 'changed helper');
    await buildShell(root);
    assert.notEqual(second, await readFile(path.join(root, 'sw.js'), 'utf8'));
    await rm(path.join(root, 'dist/snippets/hookecho/inline1-abcd5678.js'));
    await assert.rejects(buildShell(root), /ENOENT/);
  } finally { await rm(root, {recursive: true, force: true}); }
});
