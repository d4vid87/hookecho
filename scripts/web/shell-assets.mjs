import {readFile, writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {pathToFileURL} from 'node:url';

const origin = 'https://shell.invalid';
const localPath = value => {
  const url = new URL(value, origin);
  if (url.origin !== origin) throw new Error(`Shell asset is not local: ${value}`);
  return url.pathname + url.search;
};

export function pageAssets(html) {
  const assets = [...html.matchAll(/["']((?:\.\/|\/)[^\s"'<>]+\.(?:js|wasm|ttf|png|webmanifest)(?:\?[^\s"'<>]*)?)["']/g)]
    .map(match => localPath(match[1])).filter(asset => asset !== '/sw.js');
  if (!assets.some(asset => /^\/dist\/hookecho-[a-f0-9]+\.js\?boot=2$/.test(asset)) ||
      !assets.some(asset => /^\/dist\/hookecho_bg-[a-f0-9]+\.wasm$/.test(asset))) {
    throw new Error('Missing content-hashed app bundle references');
  }
  return [...new Set(['/', ...assets, '/icon-512.png'])];
}

export function glueImports(glue) {
  return [...new Set([...glue.matchAll(/^import .*? from ["']\.\/(snippets\/[^"']+\.js)["']/gm)].map(([, snippet]) => {
    if (!/inline\d+-[a-f0-9]{8}\.js$/.test(snippet)) throw new Error(`Snippet lacks a content-hashed URL: ${snippet}`);
    return localPath(`/dist/${snippet}`);
  }))];
}

// The build and deployed-asset checker follow the same module graph.
export async function shellAssets(html, read) {
  const assets = pageAssets(html);
  const glue = assets.find(asset => /^\/dist\/hookecho-[a-f0-9]+\.js\?boot=2$/.test(asset));
  return [...new Set([...assets, ...glueImports(await read(glue))])];
}

export async function buildShell(root) {
  const read = asset => readFile(path.join(root, asset === '/' ? 'index.html' : new URL(asset, origin).pathname));
  const html = await readFile(path.join(root, 'index.html'), 'utf8');
  const assets = await shellAssets(html, async asset => (await read(asset)).toString());
  const source = await readFile(path.join(root, 'sw.src.js'), 'utf8');
  const version = createHash('sha256').update(source);
  for (const asset of assets) version.update(asset).update(await read(asset));
  const worker = source.replace('__SHELL__', JSON.stringify(assets))
    .replace('__VERSION__', JSON.stringify(version.digest('hex').slice(0, 16)));
  await writeFile(path.join(root, 'sw.js'), worker);
  return assets;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const assets = await buildShell(process.argv[2] ?? 'web');
  console.log(`Offline shell: ${assets.length} assets verified`);
}
