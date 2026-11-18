// Independent S3 first-seen observations, not radar acquisition or application latency.
// SITE=KFWS SAMPLES=41 INTERVAL_MS=15000 node scripts/web/source-monitor.mjs
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';

const site = process.env.SITE ?? 'KFWS';
const samples = Number(process.env.SAMPLES ?? 41);
const intervalMs = Number(process.env.INTERVAL_MS ?? 15000);
const output = process.env.HOOKECHO_SOURCE_OUTPUT ?? 'outputs/freshness-web/source-monitor.json';
assert(/^[A-Z0-9]{4}$/.test(site));
assert(Number.isInteger(samples) && samples > 0 && intervalMs >= 1000);
const parse = xml => [...xml.matchAll(/<Contents>([\s\S]*?)<\/Contents>/g)].map(([, item]) => {
  const field = name => new RegExp(`<${name}>([\\s\\S]*?)<\\/${name}>`).exec(item)?.[1] ?? null;
  return { key: field('Key'), modified: field('LastModified'), etag: field('ETag'), bytes: Number(field('Size')) };
});
assert.equal(parse('<Contents><Key>KFWS/1/a</Key><Size>12</Size></Contents>')[0].bytes, 12);
const report = { site, started: new Date().toISOString(), intervalMs, requestedSamples: samples,
  limits: ['First-seen is bounded by listing samples and request duration; baseline objects have no absence bound.',
    'S3 LastModified is upload metadata, not acquisition time. No matching HookEcho presentation is measured.',
    'Initial volume search assumes timestamp-ordered rotating directories with directory 1 present; the next directory is checked.'],
  discovery: [], samples: [], firstSeen: [], errors: [] };
const seen = new Set();
const previous = new Map();
const next = volume => volume === 999 ? 1 : volume + 1;
async function list(volume, maxKeys = 1000) {
  const url = `https://unidata-nexrad-level2-chunks.s3.amazonaws.com/?list-type=2&prefix=${site}/${volume}/&max-keys=${maxKeys}`;
  const startUtc = new Date().toISOString(), start = performance.now();
  const response = await fetch(url, {signal: AbortSignal.timeout(12000), cache: 'no-store'});
  const headersAtMs = performance.now() - start;
  if (!response.ok) throw Error(`HTTP ${response.status}: ${url}`);
  const xml = await response.text();
  if (maxKeys === 1000) assert(!xml.includes('<IsTruncated>true</IsTruncated>'), 'volume listing truncated');
  const objects = parse(xml);
  return { url, volume, startUtc, endUtc: new Date().toISOString(), headersAtMs,
    elapsedMs: performance.now() - start, responseBytes: Buffer.byteLength(xml),
    date: response.headers.get('date'), age: response.headers.get('age'),
    lastModified: response.headers.get('last-modified'), objects };
}
async function stamp(volume) {
  const row = await list(volume, 1);
  report.discovery.push(row);
  return row.objects[0]?.modified ?? '';
}
await fs.mkdir(new URL('.', new URL(output, `file://${process.cwd()}/`)), {recursive: true});
try {
  const first = await stamp(1);
  assert(first, 'directory 1 is empty; set up a different discovery reference before sampling');
  let low = 1, high = 999, volume = 1;
  while (low <= high) {
    const mid = Math.floor((low + high) / 2);
    if (await stamp(mid) >= first) { volume = mid; low = mid + 1; }
    else high = mid - 1;
  }
  assert(await stamp(volume) >= await stamp(next(volume)), 'rotation pivot verification failed');
  for (let i = 0; i < samples; i++) {
    const start = performance.now();
    try {
      const rows = await Promise.all([list(volume), list(next(volume))]);
      for (const row of rows) {
        for (const object of row.objects) {
          const identity = `${object.key}:${object.etag}`;
          if (seen.has(identity)) continue;
          seen.add(identity);
          report.firstSeen.push({ ...object, observedAtUtc: row.endUtc,
            absentAtUtc: previous.get(row.volume) ?? null, sample: i });
        }
        previous.set(row.volume, row.startUtc);
      }
      if ((rows[1].objects[0]?.modified ?? '') > (rows[0].objects[0]?.modified ?? '')) volume = next(volume);
      report.samples.push({ index: i, rows });
    } catch (error) { report.errors.push({ sample: i, at: new Date().toISOString(), error: error.message }); }
    await fs.writeFile(output, JSON.stringify(report, null, 2));
    if (i + 1 < samples) await new Promise(resolve => setTimeout(resolve, Math.max(0, intervalMs - (performance.now() - start))));
  }
  report.finished = new Date().toISOString();
} finally { await fs.writeFile(output, JSON.stringify(report, null, 2)); }
console.log(JSON.stringify({output, samples: report.samples.length, objects: report.firstSeen.length, errors: report.errors.length}));
