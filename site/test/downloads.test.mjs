import {test} from 'node:test';
import assert from 'node:assert/strict';
import {selectDownload, downloadRedirect} from '../public/_worker.js';

const names = ['HookEcho-x86_64.AppImage', 'HookEcho-amd64.deb', 'hookecho-windows-x86_64.zip', 'HookEcho-x86_64.msi', 'HookEcho-setup-x86_64.exe', 'HookEcho-macos.zip', 'HookEcho-arm64-v8a.apk', 'SHA256SUMS'];
const release = (tag, prerelease = true) => ({tag_name: tag, prerelease, draft: false, assets: names.map(name => ({name, browser_download_url: `https://github.com/d4vid87/hookecho/releases/download/${tag}/${name}`}))});
test('downloads prefer a complete stable release and otherwise use a complete preview', () => {
  const preview = release('build-abc');
  const stable = release('v1.2.3', false);
  const name = names[0];
  assert.equal(selectDownload([preview, stable], name), stable.assets[0].browser_download_url);
  assert.equal(selectDownload([preview, {...stable, draft: true}], name), preview.assets[0].browser_download_url);
  assert.equal(selectDownload([{...preview, assets: preview.assets.slice(1)}, stable], name), stable.assets[0].browser_download_url);
  assert.equal(selectDownload([preview], '../settings.json'), null);
  assert.equal(selectDownload([{...preview, assets: preview.assets.map(a => ({...a, browser_download_url: 'https://evil.example/file'}))}], name), null);
});
test('download links survive the transition away from the legacy rolling release', () => {
  const legacy = release('latest', false);
  legacy.assets.pop();
  const name = names[0];
  assert.equal(selectDownload([legacy], name), legacy.assets[0].browser_download_url);
  const preview = release('build-new');
  assert.equal(selectDownload([legacy, preview], name), preview.assets[0].browser_download_url);
});
test('downloads honor GitHub latest over a stale list and fall back when latest is unusable', async () => {
  const original = globalThis.fetch;
  const previous = release('v1.2.3', false);
  const latest = release('build-new', false);
  try {
    for (const candidate of [latest, {...latest, assets: latest.assets.slice(1)},
      {...latest, tag_name: 'latest', assets: latest.assets.slice(0, -1)},
      {...latest, assets: latest.assets.map(a => ({...a, browser_download_url: 'https://evil.example/file'}))},
      {...latest, draft: true}, null]) {
      let listRequests = 0;
      globalThis.fetch = async url => {
        if (url.endsWith('/latest')) {
          if (!candidate) throw new Error('latest unavailable');
          return Response.json(candidate);
        }
        listRequests++;
        return Response.json([previous]);
      };
      const response = await downloadRedirect(new Request(`https://hookecho.io/download-file/${names[0]}`));
      const expected = candidate === latest ? latest : previous;
      assert.equal(response.headers.get('location'), expected.assets[0].browser_download_url);
      assert.equal(listRequests, candidate === latest ? 0 : 1);
    }
  } finally { globalThis.fetch = original; }
});
test('a GitHub outage falls back to the releases page without an open redirect', async () => {
  const original = globalThis.fetch;
  try {
    globalThis.fetch = async () => { throw new Error('offline'); };
    const response = await downloadRedirect(new Request(`https://hookecho.io/download-file/${names[0]}`));
    assert.equal(response.status, 302);
    assert.equal(response.headers.get('location'), 'https://github.com/d4vid87/hookecho/releases');
    assert.equal((await downloadRedirect(new Request('https://hookecho.io/download-file/anything'))).status, 404);
  } finally { globalThis.fetch = original; }
});
