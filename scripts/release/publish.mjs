import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';

export const requiredFiles = [
  'HookEcho-x86_64.AppImage', 'HookEcho-amd64.deb', 'hookecho-windows-x86_64.zip',
  'HookEcho-x86_64.msi', 'HookEcho-setup-x86_64.exe', 'HookEcho-macos.zip',
  'HookEcho-arm64-v8a.apk', 'HookEcho-arm64-v8a.aab',
];

export function releasePlan(ref, sha) {
  if (!/^[a-f0-9]{40}$/.test(sha ?? '')) throw new Error('Full commit SHA required');
  if (ref === 'refs/heads/main') return {tag: `build-${sha}`, title: `Preview ${sha.slice(0, 12)}`, prerelease: true};
  if (!/^refs\/tags\/v\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(ref ?? '')) throw new Error('Only main and version tags can publish');
  const tag = ref.slice('refs/tags/'.length);
  return {tag, title: tag, prerelease: tag.includes('-')};
}

export async function collectReleaseFiles(root) {
  const found = new Map();
  async function visit(dir) {
    for (const entry of await fs.readdir(dir, {withFileTypes: true})) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) await visit(file);
      else if (entry.isFile() && requiredFiles.includes(entry.name)) {
        if (found.has(entry.name)) throw new Error(`Duplicate release asset: ${entry.name}`);
        if ((await fs.stat(file)).size === 0) throw new Error(`Empty release asset: ${entry.name}`);
        found.set(entry.name, file);
      }
    }
  }
  await visit(root);
  for (const name of requiredFiles) if (!found.has(name)) throw new Error(`Missing release asset: ${name}`);
  return found;
}

async function publish() {
  const {GITHUB_REF: ref, GITHUB_SHA: sha, GITHUB_REPOSITORY: repo} = process.env;
  const plan = releasePlan(ref, sha);
  if (!repo) throw new Error('Repository required');
  const assets = await collectReleaseFiles(process.argv[2] ?? 'dist/artifacts');
  const gh = args => execFileSync('gh', args, {encoding: 'utf8'});
  let existing;
  try { existing = JSON.parse(gh(['api', `repos/${repo}/releases/tags/${encodeURIComponent(plan.tag)}`])); }
  catch (error) { if (!String(error.stderr).includes('404')) throw error; }
  const marker = `Commit: \`${sha}\``;
  if (existing && (existing.target_commitish !== sha || !existing.body?.includes(marker))) {
    throw new Error('This release belongs to a different build; refusing to overwrite it');
  }
  const retireLegacy = () => {
    let legacy;
    try { legacy = JSON.parse(gh(['api', `repos/${repo}/releases/tags/latest`])); }
    catch (error) { if (!String(error.stderr).includes('404')) throw error; }
    if (legacy && !legacy.draft && !legacy.prerelease) {
      gh(['release', 'edit', 'latest', '--repo', repo, '--prerelease', '--latest=false', '--title', 'Legacy preview (before immutable builds)']);
    }
  };
  if (existing && !existing.draft) {
    retireLegacy();
    console.log(`Already published: ${existing.html_url}`);
    return;
  }

  const stage = 'dist/publish';
  await fs.mkdir(stage, {recursive: true});
  const checksums = [];
  for (const [name, file] of assets) {
    const bytes = await fs.readFile(file);
    checksums.push(`${createHash('sha256').update(bytes).digest('hex')}  ${name}`);
    await fs.copyFile(file, path.join(stage, name));
  }
  await fs.writeFile(path.join(stage, 'SHA256SUMS'), checksums.sort().join('\n') + '\n');
  let notes = 'Tested preview of main. All platform packages come from the same commit. Older builds remain available for rollback.\n\nmacOS remains experimental.\n';
  if (ref.startsWith('refs/tags/')) {
    const changelog = await fs.readFile('CHANGELOG.md', 'utf8');
    const heading = `## ${plan.tag.slice(1)} `;
    const section = changelog.split('\n').findIndex(line => line.startsWith(heading));
    if (section < 0) throw new Error(`No changelog section for ${plan.tag}`);
    notes = changelog.split('\n').slice(section + 1).join('\n').split(/\n## /)[0].trim();
    if (!notes) throw new Error('Empty release notes');
  }
  await fs.writeFile(path.join(stage, 'notes.md'), `${notes}\n\n${marker}\n`);
  if (!existing) gh(['release', 'create', plan.tag, '--repo', repo, '--target', sha, '--draft', '--title', plan.title, '--notes-file', path.join(stage, 'notes.md')]);
  gh(['release', 'upload', plan.tag, '--repo', repo, '--clobber', ...requiredFiles.map(name => path.join(stage, name)), path.join(stage, 'SHA256SUMS')]);
  gh(['release', 'edit', plan.tag, '--repo', repo, '--draft=false', `--prerelease=${plan.prerelease}`, `--latest=${!plan.prerelease}`]);
  // Retire the old mutable channel without removing its last downloadable packages.
  retireLegacy();
  console.log(`Published complete release: https://github.com/${repo}/releases/tag/${plan.tag}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await publish();
