// Announce a release: post to the wired channels, write drafts for the ones that are hand-posted.
//
//   node scripts/announce/post.mjs v0.9.0 [--dry-run]
//
// Bluesky, Mastodon, X and a Discord webhook are posted automatically; each is skipped with a log
// line if its secrets are unset, so a half-configured repo still works. Reddit and Hacker News are
// never automated — both communities read that as spam, and both are where the traffic is — so
// this writes announce-drafts.md to paste by hand (see docs/promotion.md).
//
// ponytail: zero dependencies (fetch + node:crypto, node >= 20), text and links only. No image
// upload: the repo's social-preview image already gives every channel a link card, and hero.gif is
// far over Bluesky's blob cap. Upload code is the upgrade path if a plain card ever underperforms.
import { createHmac } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const REPO = "d4vid87/hookecho";
const DEMO = "https://hookecho.pages.dev/";
const tag = process.argv[2];
const dryRun = process.argv.includes("--dry-run");
if (!tag) {
  console.error("usage: post.mjs <tag> [--dry-run]");
  process.exit(1);
}
const version = tag.replace(/^v/, "");
// The tagged release URL, not the asset: `demote-old` in release.yml drafts old releases, so this
// link rots one release later. That is why the demo is the primary call to action everywhere.
const releaseUrl = `https://github.com/${REPO}/releases/tag/${tag}`;

/** The tag's own CHANGELOG section, as a bullet list. */
function notes() {
  const lines = readFileSync("CHANGELOG.md", "utf8").split("\n");
  const start = lines.findIndex((l) => l.startsWith(`## ${version} `));
  if (start < 0) throw new Error(`no CHANGELOG.md section for ${version}`);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((l) => l.startsWith("## "));
  return (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
}

/** The bullets as one-line summaries, longest-first trimmed to fit a budget. */
function highlights(body, max) {
  const items = body
    .split(/\n(?=- )/)
    .map((b) => b.replace(/^- /, "").replace(/\s+/g, " ").trim())
    .filter(Boolean);
  const out = [];
  let used = 0;
  for (const item of items) {
    const line = `• ${item.split(" — ")[0].replace(/[.:,;]$/, "")}`;
    if (used + line.length + 1 > max) break;
    out.push(line);
    used += line.length + 1;
  }
  return out.join("\n");
}

const body = notes();
const headline = `HookEcho ${version} is out — an open-source NEXRAD radar viewer in Rust (wgpu + egui). No accounts, no telemetry, runs on your machine.`;

function compose(limit, links) {
  const tail = `\n\n${links}`;
  const room = limit - headline.length - tail.length - 2;
  const bullets = room > 40 ? highlights(body, room) : "";
  return `${headline}${bullets ? `\n\n${bullets}` : ""}${tail}`;
}

// --- channels ---------------------------------------------------------------

async function postBluesky(text) {
  const { BSKY_HANDLE, BSKY_APP_PASSWORD } = process.env;
  if (!BSKY_HANDLE || !BSKY_APP_PASSWORD) return skip("bluesky");
  const api = "https://bsky.social/xrpc";
  const session = await json(`${api}/com.atproto.server.createSession`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ identifier: BSKY_HANDLE, password: BSKY_APP_PASSWORD }),
  });
  await json(`${api}/com.atproto.repo.createRecord`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${session.accessJwt}`,
    },
    body: JSON.stringify({
      repo: session.did,
      collection: "app.bsky.feed.post",
      record: {
        $type: "app.bsky.feed.post",
        text,
        facets: linkFacets(text),
        createdAt: new Date().toISOString(),
      },
    }),
  });
  console.log("bluesky: posted");
}

/** Bluesky wants byte offsets into the UTF-8 encoding, not character indices. */
function linkFacets(text) {
  const bytes = Buffer.from(text, "utf8");
  const facets = [];
  for (const m of text.matchAll(/https?:\/\/\S+/g)) {
    const uri = m[0].replace(/[.,)]+$/, "");
    const byteStart = Buffer.from(text.slice(0, m.index), "utf8").length;
    facets.push({
      index: { byteStart, byteEnd: byteStart + Buffer.from(uri, "utf8").length },
      features: [{ $type: "app.bsky.richtext.facet#link", uri }],
    });
  }
  return bytes.length ? facets : [];
}

async function postMastodon(text) {
  const { MASTODON_URL, MASTODON_TOKEN } = process.env;
  if (!MASTODON_URL || !MASTODON_TOKEN) return skip("mastodon");
  await json(`${MASTODON_URL.replace(/\/$/, "")}/api/v1/statuses`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${MASTODON_TOKEN}`,
    },
    body: JSON.stringify({ status: text }),
  });
  console.log("mastodon: posted");
}

async function postX(text) {
  const { X_API_KEY, X_API_SECRET, X_ACCESS_TOKEN, X_ACCESS_SECRET } = process.env;
  if (!X_API_KEY || !X_API_SECRET || !X_ACCESS_TOKEN || !X_ACCESS_SECRET) return skip("x");
  const url = "https://api.twitter.com/2/tweets";
  const auth = oauth1Header("POST", url, {
    consumerKey: X_API_KEY,
    consumerSecret: X_API_SECRET,
    token: X_ACCESS_TOKEN,
    tokenSecret: X_ACCESS_SECRET,
  });
  await json(url, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: auth },
    body: JSON.stringify({ text }),
  });
  console.log("x: posted");
}

// OAuth 1.0a, HMAC-SHA1, for one JSON endpoint whose body is not signed.
// ponytail: 40 lines beats a dependency for a single call — reach for twitter-api-v2 the day a
// second endpoint (media upload) shows up.
function oauth1Header(method, url, keys, extra = {}) {
  const enc = (s) =>
    encodeURIComponent(s).replace(/[!*'()]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
  const params = {
    oauth_consumer_key: keys.consumerKey,
    oauth_nonce: randomNonce(),
    oauth_signature_method: "HMAC-SHA1",
    oauth_timestamp: String(Math.floor(Date.now() / 1000)),
    oauth_token: keys.token,
    oauth_version: "1.0",
    ...extra,
  };
  const base = [
    method.toUpperCase(),
    enc(url),
    enc(
      Object.keys(params)
        .sort()
        .map((k) => `${enc(k)}=${enc(params[k])}`)
        .join("&"),
    ),
  ].join("&");
  const signingKey = `${enc(keys.consumerSecret)}&${enc(keys.tokenSecret)}`;
  params.oauth_signature = createHmac("sha1", signingKey).update(base).digest("base64");
  return (
    "OAuth " +
    Object.keys(params)
      .sort()
      .map((k) => `${enc(k)}="${enc(params[k])}"`)
      .join(", ")
  );
}

function randomNonce() {
  return createHmac("sha1", String(Date.now())).update(String(Math.random())).digest("hex");
}

async function postDiscord(text) {
  const url = process.env.DISCORD_WEBHOOK_URL;
  if (!url) return skip("discord");
  const resp = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ content: text }),
  });
  if (!resp.ok) throw new Error(`discord ${resp.status}: ${await resp.text()}`);
  console.log("discord: posted");
}

const skip = (name) => console.log(`${name}: skipped (secrets unset)`);

async function json(url, init) {
  const resp = await fetch(url, init);
  const text = await resp.text();
  if (!resp.ok) throw new Error(`${url} ${resp.status}: ${text.slice(0, 300)}`);
  return text ? JSON.parse(text) : {};
}

// --- hand-posted drafts -----------------------------------------------------

function makeDrafts() {
  const bullets = body;
  const sub = (name, title, intro) =>
    `### r/${name}\n\n**Title:** ${title}\n\n${intro}\n\n${bullets}\n\nLive demo (runs in the browser, no install): ${DEMO}\nSource and downloads: https://github.com/${REPO}\n`;

  return `# Announcement drafts — ${tag}

Paste these by hand, one community per day, in the wave order in docs/promotion.md.
Reddit and HN are deliberately not automated. Reply to everything for the first
6–12 hours; that is the part that decides how the post does.

${sub(
  "rust",
  `HookEcho ${version} — NEXRAD weather radar in Rust, wgpu + egui, also compiled to wasm`,
  "Written from scratch in Rust: Level 2/3 radar decode, a wgpu render pipeline, egui UI, and the same codebase running as a desktop app, an Android APK and a wasm build in the browser. Happy to talk about the rendering or the decoding.",
)}
${sub(
  "meteorology",
  `HookEcho ${version} — a free, open-source radar viewer with soundings, effective-layer parameters and model fields`,
  "Free and open source, no account or subscription. Level 2/3 per-site analysis, archive replay back to 1991, forecast soundings with effective-layer severe parameters, HRRR/RAP/GFS/ECMWF fields and a model difference layer.",
)}
${sub(
  "stormchasing",
  `HookEcho ${version} — open-source radar for chasing: archive replay, spotter network, rain-arrival ETA, offline chase packs`,
  "Built for the road: offline chase packs, GPS position sharing through your own relay, spotter network overlay, warning alerts read aloud, rain-arrival ETA, and Android as well as desktop. No subscription.",
)}
${sub(
  "opensource",
  `HookEcho ${version} — MIT-licensed NEXRAD radar viewer, no accounts, no telemetry`,
  "MIT, Rust, no hosted service and no telemetry — the app talks to public NOAA/NWS feeds from your own machine. Linux AppImage, Windows installer, Android APK, container image, and a browser build.",
)}
## Hacker News (Show HN)

**Title:** Show HN: HookEcho – NEXRAD weather radar viewer in Rust (${DEMO})

**First comment (post it immediately after submitting):**

I built this because the good radar software is Windows-only, paid, or both. It
decodes NEXRAD Level 2/3 itself, renders through wgpu, and the same code runs on
the desktop, on Android and in the browser as wasm — the demo link is the real
app, streaming live chunks as a scan comes in. No accounts, no telemetry, no
hosted service: it talks to the public NOAA and NWS feeds from your machine.

${bullets}

Source: https://github.com/${REPO}
`;
}

// --- run --------------------------------------------------------------------

const texts = {
  bluesky: compose(300, `${DEMO}\n${releaseUrl}`),
  mastodon: compose(500, `Live demo: ${DEMO}\nRelease: ${releaseUrl}`),
  x: compose(280, DEMO),
  discord: compose(2000, `Live demo: ${DEMO}\nRelease: ${releaseUrl}`),
};

writeFileSync("announce-drafts.md", makeDrafts());
console.log("wrote announce-drafts.md");

if (dryRun) {
  selfCheckOauth1();
  for (const [name, text] of Object.entries(texts)) {
    console.log(`\n--- ${name} (${text.length} chars) ---\n${text}`);
  }
  console.log(`\n--- drafts ---\n${makeDrafts()}`);
  process.exit(0);
}

// One channel refusing is not a failed announcement — X in particular answers 402 the moment its
// pay-per-use credits run out, and that must not take the other channels or the drafts with it.
// The run still fails at the end so the refusal is visible rather than buried in a green log.
const failed = [];
for (const [name, post, text] of [
  ["bluesky", postBluesky, texts.bluesky],
  ["mastodon", postMastodon, texts.mastodon],
  ["x", postX, texts.x],
  ["discord", postDiscord, texts.discord],
]) {
  try {
    await post(text);
  } catch (e) {
    failed.push(name);
    console.error(`::warning::${name}: ${e.message}`);
  }
}
if (failed.length) {
  console.error(`::error::channels that refused: ${failed.join(", ")}`);
  process.exitCode = 1;
}

// The signature is the one piece here that fails silently-but-authentically if it is subtly wrong,
// so it is checked against RFC 5849 §3.1's own worked example.
function selfCheckOauth1() {
  const header = oauth1Header(
    "POST",
    "http://example.com/request",
    {
      consumerKey: "9djdj82h48djs9d2",
      consumerSecret: "j49sk3j29djd",
      token: "kkk9d7dh3k39sjv7",
      tokenSecret: "dh893hdasih9",
    },
    { oauth_timestamp: "137131201", oauth_nonce: "7d8f3e4a" },
  );
  if (!/oauth_signature="[^"]+"/.test(header)) throw new Error("oauth1: no signature produced");
  if (!header.includes('oauth_signature_method="HMAC-SHA1"')) throw new Error("oauth1: bad header");
  console.log("oauth1 self-check: header well-formed");
}
