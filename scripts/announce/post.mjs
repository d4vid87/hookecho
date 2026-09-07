// One no-dependency promotion entry point: prepare factual campaigns, then publish one channel.
// Public posting is disabled for scheduled/weather work until PROMOTION_ENABLED=true. Stable
// HookEcho releases keep the behaviour they had before this system existed.
import { appendFileSync, readFileSync, statSync, writeFileSync } from "node:fs";
import {
  CHANNELS,
  PRODUCTS,
  PROMOTION_START,
  campaignForDate,
  composeText,
  isPaused,
  pickWeatherCampaign,
  releaseCampaign,
  trackedUrl,
  weatherCapAllows,
} from "./campaigns.mjs";

const UA = "HookEcho promotion (https://github.com/d4vid87/hookecho)";
const GRAPH = process.env.META_GRAPH_VERSION || "v25.0";

export function markerName(campaign, channel) {
  return `promo-${campaign.id}-${channel}`.replace(/[^A-Za-z0-9_.-]/g, "-").slice(0, 240);
}

export function channelSecrets(channel, product, env = process.env) {
  const suffix = product.toUpperCase();
  if (channel === "bluesky") return env.BSKY_HANDLE && env.BSKY_APP_PASSWORD;
  if (channel === "mastodon") return env.MASTODON_URL && env.MASTODON_TOKEN;
  if (channel === "discord") return env.DISCORD_WEBHOOK_URL;
  if (channel === "youtube") return env.GOOGLE_OAUTH_CLIENT_ID && env.GOOGLE_OAUTH_CLIENT_SECRET && env[`YOUTUBE_${suffix}_REFRESH_TOKEN`];
  if (channel === "facebook") return env[`META_${suffix}_PAGE_ID`] && env[`META_${suffix}_PAGE_TOKEN`];
  if (channel === "instagram") return env[`META_${suffix}_IG_USER_ID`] && (env[`META_${suffix}_IG_TOKEN`] || env[`META_${suffix}_PAGE_TOKEN`]);
  return false;
}

export async function retryingFetch(url, init, fetchFn = fetch, sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))) {
  let last;
  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      const response = await fetchFn(url, init);
      if (response.ok || (response.status !== 429 && response.status < 500)) return response;
      last = new Error(`${response.status} ${await response.text()}`);
      last.status = response.status;
    } catch (error) {
      last = error;
    }
    if (attempt < 3) await sleep(500 * 2 ** attempt);
  }
  throw last;
}

async function request(url, init = {}, fetchFn = fetch) {
  const response = await retryingFetch(url, {
    ...init,
    headers: { "user-agent": UA, ...(init.headers || {}) },
  }, fetchFn);
  const body = await response.text();
  if (!response.ok) {
    const error = new Error(`${response.status} ${url}: ${body.slice(0, 300)}`);
    error.status = response.status;
    throw error;
  }
  return body ? JSON.parse(body) : {};
}

async function combinedStars() {
  const values = await Promise.all(Object.values(PRODUCTS).map(async ({ repo }) => {
    try {
      return (await request(`https://api.github.com/repos/${repo}`, githubHeaders())).stargazers_count || 0;
    } catch {
      return 0;
    }
  }));
  return values.reduce((sum, value) => sum + value, 0);
}

function githubHeaders() {
  return process.env.GITHUB_TOKEN
    ? { headers: { authorization: `Bearer ${process.env.GITHUB_TOKEN}`, accept: "application/vnd.github+json" } }
    : {};
}

async function listArtifacts() {
  const { GITHUB_TOKEN, GITHUB_REPOSITORY } = process.env;
  if (!GITHUB_TOKEN || !GITHUB_REPOSITORY) return [];
  const artifacts = [];
  for (let page = 1; page <= 5; page++) {
    const data = await request(
      `https://api.github.com/repos/${GITHUB_REPOSITORY}/actions/artifacts?per_page=100&page=${page}`,
      githubHeaders(),
    );
    artifacts.push(...(data.artifacts || []));
    if ((data.artifacts || []).length < 100) break;
  }
  return artifacts.filter((artifact) => !artifact.expired);
}

async function prepareScheduled(output, date = new Date().toISOString(), saturday) {
  const artifacts = await listArtifacts();
  const selected = [saturday, latestSaturdayProduct(artifacts), process.env.SATURDAY_PRODUCT].find((value) => PRODUCTS[value]) || "hookecho";
  const campaign = campaignForDate(date, selected);
  if (!campaign) return false;
  campaign.combinedStars = await combinedStars();
  writeFileSync(output, JSON.stringify(campaign, null, 2));
  return true;
}

async function prepareRelease(output, product, tag, changelogPath) {
  const changelog = readFileSync(changelogPath, "utf8");
  const campaign = releaseCampaign(product, tag, changelog);
  campaign.combinedStars = await combinedStars();
  writeFileSync(output, JSON.stringify(campaign, null, 2));
  return true;
}

async function prepareLatestRelease(output, product) {
  if (!PRODUCTS[product]) throw new Error("unknown product");
  const release = await request(`https://api.github.com/repos/${PRODUCTS[product].repo}/releases/latest`, githubHeaders());
  if (release.draft || release.prerelease || !release.tag_name || new Date(release.published_at) < new Date(`${PROMOTION_START}T00:00:00Z`)) return false;
  const version = release.tag_name.replace(/^v/, "");
  const heading = product === "stormdesk" ? `## [${version}]` : `## ${version}`;
  const campaign = releaseCampaign(product, release.tag_name, `${heading}\n${release.body || "- A new stable release is ready."}`);
  campaign.combinedStars = await combinedStars();
  writeFileSync(output, JSON.stringify(campaign, null, 2));
  return true;
}

async function prepareWeather(output) {
  const artifacts = await listArtifacts();
  const today = new Date().toISOString().slice(0, 10);
  const monitorName = `weather-monitor-${today}`;
  writeOutput("monitor_marker", artifacts.some((artifact) => artifact.name === monitorName) ? "" : monitorName);

  const nws = await request("https://api.weather.gov/alerts/active?status=actual&message_type=alert", {
    headers: { accept: "application/geo+json" },
  });
  if (!Array.isArray(nws.features)) throw new Error("NWS returned a malformed alert feed");
  const nhcUrls = ["index-at.xml", "index-ep.xml", "index-cp.xml"].map((name) => `https://www.nhc.noaa.gov/${name}`);
  const nhc = await Promise.all(nhcUrls.map(async (url) => {
    try {
      const response = await retryingFetch(url, { headers: { "user-agent": UA } });
      return response.ok ? response.text() : "";
    } catch {
      return "";
    }
  }));
  if (nhc.some((xml) => !/<rss\b/i.test(xml) || !/<\/rss>/i.test(xml))) throw new Error("NHC returned a missing or malformed feed");

  const monitorDays = new Set(
    artifacts.map((artifact) => artifact.name.match(/^weather-monitor-(\d{4}-\d{2}-\d{2})$/)?.[1]).filter(Boolean),
  );
  monitorDays.add(today);
  writeOutput("monitor_days", String(monitorDays.size));
  if (monitorDays.size < 7) return false;

  const published = publishedWeather(artifacts);
  if (!weatherCapAllows(published)) return false;
  const ids = new Set(published.map((item) => item.id));
  const campaign = pickWeatherCampaign(nws.features, nhc, ids);
  if (!campaign) return false;
  campaign.combinedStars = await combinedStars();
  writeFileSync(output, JSON.stringify(campaign, null, 2));
  return true;
}

function publishedWeather(artifacts) {
  const found = new Map();
  for (const artifact of artifacts) {
    const match = artifact.name.match(/^promo-(weather-[a-f0-9]{10}-[a-f0-9]{10})-(?:bluesky|mastodon|facebook)$/);
    if (match && !found.has(match[1])) found.set(match[1], { id: match[1], createdAt: artifact.created_at });
  }
  return [...found.values()];
}

async function publishFile(path, channel, media, dryRun = false) {
  const campaign = JSON.parse(readFileSync(path, "utf8"));
  if (!PRODUCTS[campaign.product] || !campaign.id || !campaign.title || !campaign.body) throw new Error("invalid campaign file");
  if (![...CHANNELS, "discord"].includes(channel)) throw new Error("unknown channel");
  const marker = markerName(campaign, channel);
  writeOutput("marker", marker);

  if (campaign.kind === "weather" && !["bluesky", "mastodon", "facebook"].includes(channel)) return skipped(channel, "weather is text-only");
  if (["youtube", "instagram"].includes(channel) && !media) return skipped(channel, "media missing");
  const text = composeText(campaign, channel);
  if (dryRun || process.env.DRY_RUN === "true") {
    console.log(`[dry-run] ${channel} ${marker}\n${text}`);
    return false;
  }

  const artifacts = await listArtifacts();
  if (isPaused(channel, process.env.PROMOTION_PAUSES) || artifactPaused(channel, artifacts)) return skipped(channel, "paused");
  if (artifacts.some((artifact) => artifact.name === marker)) return skipped(channel, "already posted");
  if (!channelSecrets(channel, campaign.product)) return skipped(channel, "secrets unset");
  if (campaign.kind !== "release" && process.env.PROMOTION_ENABLED !== "true" && process.env.MANUAL_TEST !== "true") return skipped(channel, "promotion disabled");
  if (channel === "youtube" && process.env.YOUTUBE_PUBLIC_UPLOADS !== "true" && process.env.YOUTUBE_TEST_UPLOADS !== "true") {
    return skipped(channel, "YouTube audit not enabled");
  }

  try {
    if (channel === "bluesky") await postBluesky(text);
    else if (channel === "mastodon") await postMastodon(text);
    else if (channel === "discord") await postDiscord(text);
    else if (channel === "youtube") await postYouTube(campaign, text, media);
    else if (channel === "facebook") await postFacebook(campaign, text, media);
    else if (channel === "instagram") await postInstagram(campaign, text, media);
  } catch (error) {
    writeFileSync("promotion-failure.txt", `${new Date().toISOString()} ${campaign.id} ${channel} ${error.status || "error"}\n`);
    const kind = error.status === 401 || error.status === 403 ? "promo-auth-failure" : "promo-failure";
    writeOutput("failure_marker", `${kind}-${channel}-${Date.now()}`);
    throw error;
  }
  writeOutput("posted", "true");
  console.log(`${channel}: posted ${campaign.id}`);
  return true;
}

function latestSaturdayProduct(artifacts) {
  return [...artifacts]
    .sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)))
    .map(({ name }) => name.match(/^promotion-saturday-(hookecho|stormdesk)-/)?.[1])
    .find(Boolean);
}

function artifactPaused(channel, artifacts) {
  return artifacts.some(({ name, created_at }) =>
    name.startsWith("promotion-pauses-")
    && name.split("-").includes(channel)
    && Date.now() - new Date(created_at).valueOf() < 14 * 86_400_000,
  );
}

async function postBluesky(text) {
  const api = "https://bsky.social/xrpc";
  const session = await request(`${api}/com.atproto.server.createSession`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ identifier: process.env.BSKY_HANDLE, password: process.env.BSKY_APP_PASSWORD }),
  });
  await request(`${api}/com.atproto.repo.createRecord`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${session.accessJwt}` },
    body: JSON.stringify({
      repo: session.did,
      collection: "app.bsky.feed.post",
      record: { $type: "app.bsky.feed.post", text, facets: linkFacets(text), createdAt: new Date().toISOString() },
    }),
  });
}

export function linkFacets(text) {
  return [...text.matchAll(/https?:\/\/\S+/g)].map((match) => {
    const uri = match[0].replace(/[.,)]+$/, "");
    const byteStart = Buffer.byteLength(text.slice(0, match.index));
    return {
      index: { byteStart, byteEnd: byteStart + Buffer.byteLength(uri) },
      features: [{ $type: "app.bsky.richtext.facet#link", uri }],
    };
  });
}

async function postMastodon(text) {
  await request(`${process.env.MASTODON_URL.replace(/\/$/, "")}/api/v1/statuses`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${process.env.MASTODON_TOKEN}` },
    body: JSON.stringify({ status: text }),
  });
}

async function postDiscord(text) {
  await request(process.env.DISCORD_WEBHOOK_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ content: text }),
  });
}

async function googleToken(product) {
  const suffix = product.toUpperCase();
  const body = new URLSearchParams({
    client_id: process.env.GOOGLE_OAUTH_CLIENT_ID,
    client_secret: process.env.GOOGLE_OAUTH_CLIENT_SECRET,
    refresh_token: process.env[`YOUTUBE_${suffix}_REFRESH_TOKEN`],
    grant_type: "refresh_token",
  });
  return (await request("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body,
  })).access_token;
}

async function postYouTube(campaign, text, media) {
  const token = await googleToken(campaign.product);
  const size = statSync(media).size;
  const privacyStatus = process.env.YOUTUBE_PUBLIC_UPLOADS === "true" ? "public" : "private";
  const response = await retryingFetch(
    "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status",
    {
      method: "POST",
      headers: {
        "user-agent": UA,
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
        "x-upload-content-length": String(size),
        "x-upload-content-type": "video/mp4",
      },
      body: JSON.stringify({
        snippet: { title: campaign.title.slice(0, 100), description: text, categoryId: "28" },
        status: { privacyStatus, selfDeclaredMadeForKids: false },
      }),
    },
  );
  if (!response.ok) throw Object.assign(new Error(`YouTube ${response.status}: ${(await response.text()).slice(0, 300)}`), { status: response.status });
  const upload = response.headers.get("location");
  if (!upload) throw new Error("YouTube returned no upload URL");
  const done = await retryingFetch(upload, {
    method: "PUT",
    headers: { authorization: `Bearer ${token}`, "content-type": "video/mp4", "content-length": String(size) },
    body: readFileSync(media),
  });
  if (!done.ok) throw Object.assign(new Error(`YouTube upload ${done.status}: ${(await done.text()).slice(0, 300)}`), { status: done.status });
}

function metaCredentials(product, instagram = false) {
  const suffix = product.toUpperCase();
  return instagram
    ? { id: process.env[`META_${suffix}_IG_USER_ID`], token: process.env[`META_${suffix}_IG_TOKEN`] || process.env[`META_${suffix}_PAGE_TOKEN`] }
    : { id: process.env[`META_${suffix}_PAGE_ID`], token: process.env[`META_${suffix}_PAGE_TOKEN`] };
}

async function postFacebook(campaign, text, media) {
  const { id, token } = metaCredentials(campaign.product);
  if (!media) {
    await request(`https://graph.facebook.com/${GRAPH}/${id}/feed`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ message: text, access_token: token }),
    });
    return;
  }
  const start = await request(`https://graph.facebook.com/${GRAPH}/${id}/video_reels`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ upload_phase: "start", access_token: token }),
  });
  const bytes = readFileSync(media);
  const upload = await retryingFetch(start.upload_url, {
    method: "POST",
    headers: { authorization: `OAuth ${token}`, offset: "0", file_size: String(bytes.length), "content-type": "application/octet-stream" },
    body: bytes,
  });
  if (!upload.ok) throw Object.assign(new Error(`Facebook upload ${upload.status}: ${(await upload.text()).slice(0, 300)}`), { status: upload.status });
  await request(`https://graph.facebook.com/${GRAPH}/${id}/video_reels`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      upload_phase: "finish",
      video_id: String(start.video_id),
      video_state: "PUBLISHED",
      description: text,
      access_token: token,
    }),
  });
}

async function postInstagram(campaign, text, media) {
  const { id, token } = metaCredentials(campaign.product, true);
  const container = await request(`https://graph.facebook.com/${GRAPH}/${id}/media`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ media_type: "REELS", upload_type: "resumable", caption: text, access_token: token }),
  });
  const bytes = readFileSync(media);
  const uploaded = await retryingFetch(container.uri, {
    method: "POST",
    headers: { authorization: `OAuth ${token}`, offset: "0", file_size: String(bytes.length), "content-type": "application/octet-stream" },
    body: bytes,
  });
  if (!uploaded.ok) throw Object.assign(new Error(`Instagram upload ${uploaded.status}: ${(await uploaded.text()).slice(0, 300)}`), { status: uploaded.status });
  for (let attempt = 0; attempt < 24; attempt++) {
    const status = await request(`https://graph.facebook.com/${GRAPH}/${container.id}?fields=status_code&access_token=${encodeURIComponent(token)}`);
    if (status.status_code === "FINISHED") break;
    if (status.status_code === "ERROR" || status.status_code === "EXPIRED") throw new Error(`Instagram container ${status.status_code}`);
    if (attempt === 23) throw new Error("Instagram processing timed out");
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  await request(`https://graph.facebook.com/${GRAPH}/${id}/media_publish`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ creation_id: String(container.id), access_token: token }),
  });
}

function skipped(channel, reason) {
  console.log(`${channel}: skipped (${reason})`);
  writeOutput("posted", "false");
  return false;
}

function writeOutput(name, value) {
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`);
}

async function notify(text) {
  if (!process.env.DISCORD_WEBHOOK_URL) return console.log(`discord: skipped (${text})`);
  await postDiscord(text.slice(0, 1900));
}

async function main(args = process.argv.slice(2)) {
  const [command, ...rest] = args;
  let made = false;
  if (command === "prepare-scheduled") made = await prepareScheduled(rest[0], rest[1], rest[2]);
  else if (command === "prepare-release") made = await prepareRelease(rest[0], rest[1], rest[2], rest[3]);
  else if (command === "prepare-latest-release") made = await prepareLatestRelease(rest[0], rest[1]);
  else if (command === "prepare-weather") made = await prepareWeather(rest[0]);
  else if (command === "publish") return publishFile(rest[0], rest[1], rest[2] || "", rest.includes("--dry-run"));
  else if (command === "notify") return notify(rest.join(" "));
  else throw new Error("usage: post.mjs prepare-scheduled|prepare-release|prepare-latest-release|prepare-weather|publish|notify …");
  writeOutput("has_campaign", String(made));
  if (made) console.log(`prepared ${JSON.parse(readFileSync(rest[0], "utf8")).id}`);
}

const invoked = process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href;
if (invoked) main().catch((error) => {
  console.error(`::error::${error.message}`);
  process.exitCode = 1;
});

export { main, prepareLatestRelease, prepareRelease, prepareScheduled, prepareWeather, publishFile, trackedUrl };
