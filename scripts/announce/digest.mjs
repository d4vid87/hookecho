import { appendFileSync } from "node:fs";
import { promotionPauses } from "./campaigns.mjs";

// Weekly aggregate promotion report and the two small feedback controls used by Saturday posts.
// No usernames, IPs, referrers tied to people, or other identifiers are stored or emitted.
const PRODUCTS = [
  { id: "hookecho", repo: "d4vid87/hookecho", youtube: "YOUTUBE_HOOKECHO_REFRESH_TOKEN", page: "META_HOOKECHO_PAGE_ID", pageToken: "META_HOOKECHO_PAGE_TOKEN", ig: "META_HOOKECHO_IG_USER_ID", igToken: "META_HOOKECHO_IG_TOKEN" },
  { id: "stormdesk", repo: "d4vid87/stormdesk", youtube: "YOUTUBE_STORMDESK_REFRESH_TOKEN", page: "META_STORMDESK_PAGE_ID", pageToken: "META_STORMDESK_PAGE_TOKEN", ig: "META_STORMDESK_IG_USER_ID", igToken: "META_STORMDESK_IG_TOKEN" },
];
const SOCIAL_CHANNELS = new Set(["bluesky", "mastodon", "youtube", "facebook", "instagram"]);
const SINCE = Date.now() - 7 * 86_400_000;
const UA = "HookEcho promotion digest (https://github.com/d4vid87/hookecho)";
const sections = [];
const failures = [];

async function section(name, fn) {
  try {
    const lines = await fn();
    if (lines.length) sections.push(`**${name}**\n${lines.join("\n")}`);
  } catch (error) {
    failures.push(`${name}: ${error.message}`);
  }
}

async function fetchText(url, init = {}) {
  const response = await fetch(url, { ...init, headers: { "user-agent": UA, ...(init.headers || {}) } });
  const text = await response.text();
  if (!response.ok) throw new Error(`${response.status}`);
  return text;
}

const get = async (url, init = {}) => JSON.parse(await fetchText(url, init));
const github = (accept = "application/vnd.github+json") => ({
  headers: { accept, ...(process.env.GITHUB_TOKEN ? { authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {}) },
});

async function repoMetrics(product) {
  const [repo, releases, views, clones, referrers, stars] = await Promise.all([
    get(`https://api.github.com/repos/${product.repo}`, github()),
    get(`https://api.github.com/repos/${product.repo}/releases?per_page=100`, github()),
    get(`https://api.github.com/repos/${product.repo}/traffic/views`, github()).catch(() => ({})),
    get(`https://api.github.com/repos/${product.repo}/traffic/clones`, github()).catch(() => ({})),
    get(`https://api.github.com/repos/${product.repo}/traffic/popular/referrers`, github()).catch(() => []),
    getStars(product.repo).catch(() => []),
  ]);
  return {
    id: product.id,
    stars: repo.stargazers_count,
    starDelta: stars.filter((star) => new Date(star.starred_at).valueOf() > SINCE).length,
    downloads: releases.flatMap((release) => release.assets || []).reduce((sum, asset) => sum + asset.download_count, 0),
    visitors: views.uniques || 0,
    cloners: clones.uniques || 0,
    referrers: (referrers || []).slice(0, 3).map((row) => `${row.referrer} ${row.uniques}`).join(", "),
  };
}

async function getStars(repo) {
  const all = [];
  for (let page = 1; page <= 5; page++) {
    const rows = await get(`https://api.github.com/repos/${repo}/stargazers?per_page=100&page=${page}`, github("application/vnd.github.star+json"));
    all.push(...rows);
    if (rows.length < 100) break;
  }
  return all;
}

await section("Mentions", async () => {
  const hits = [];
  for (const query of ["hookecho", "stormdesk"]) {
    const data = await get(`https://hn.algolia.com/api/v1/search_by_date?query=${query}&numericFilters=created_at_i>${Math.floor(SINCE / 1000)}`);
    hits.push(...data.hits.slice(0, 3).map((hit) => `• HN: ${hit.title || hit.story_title} — https://news.ycombinator.com/item?id=${hit.objectID}`));
    const githubMentions = await get(`https://api.github.com/search/issues?q=${query}+in:title,body+-repo:d4vid87/hookecho+-repo:d4vid87/stormdesk+updated:>${new Date(SINCE).toISOString().slice(0, 10)}&per_page=3`, github());
    hits.push(...(githubMentions.items || []).map((item) => `• GitHub: ${item.title} — ${item.html_url}`));
  }
  if (process.env.BSKY_HANDLE && process.env.BSKY_APP_PASSWORD) {
    const session = await get("https://bsky.social/xrpc/com.atproto.server.createSession", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ identifier: process.env.BSKY_HANDLE, password: process.env.BSKY_APP_PASSWORD }),
    });
    for (const query of ["hookecho", "stormdesk"]) {
      const data = await get(`https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=${query}&limit=25`, {
        headers: { authorization: `Bearer ${session.accessJwt}` },
      });
      hits.push(...(data.posts || [])
        .filter((post) => new Date(post.indexedAt).valueOf() > SINCE)
        .slice(0, 3)
        .map((post) => `• Bluesky @${post.author.handle}: ${(post.record.text || "").replace(/\s+/g, " ").slice(0, 100)}`));
    }
  }
  return [...new Set(hits)];
});

const productMetrics = [];
await section("GitHub", async () => {
  productMetrics.push(...await Promise.all(PRODUCTS.map(repoMetrics)));
  return productMetrics.map((item) =>
    `• ${item.id === "hookecho" ? "HookEcho" : "StormDesk"}: ${item.stars} stars (+${item.starDelta}), ${item.visitors} visitors, ${item.cloners} cloners, ${item.downloads} downloads; refs: ${item.referrers || "none"}`,
  );
});

const cta = await ctaMetrics().catch((error) => {
  failures.push(`CTA analytics: ${error.message}`);
  return [];
});
await section("Tracked clicks", async () => cta.slice(0, 12).map((row) => `• ${row.target}/${row.placement}: ${Math.round(row.clicks)}`));

await section("YouTube", async () => {
  const lines = [];
  for (const product of PRODUCTS) {
    if (!process.env[product.youtube]) continue;
    const token = await googleToken(process.env[product.youtube]);
    const startDate = new Date(SINCE).toISOString().slice(0, 10);
    const endDate = new Date().toISOString().slice(0, 10);
    const url = new URL("https://youtubeanalytics.googleapis.com/v2/reports");
    url.search = new URLSearchParams({
      ids: "channel==MINE",
      startDate,
      endDate,
      metrics: "views,averageViewDuration,shares,subscribersGained",
    });
    const report = await get(url, { headers: { authorization: `Bearer ${token}` } });
    const values = Object.fromEntries((report.columnHeaders || []).map((column, i) => [column.name, report.rows?.[0]?.[i] || 0]));
    lines.push(`• ${product.id}: ${values.views || 0} views, ${Math.round(values.averageViewDuration || 0)}s average, ${values.shares || 0} shares, +${values.subscribersGained || 0} subscribers`);
  }
  return lines;
});

await section("Meta", async () => {
  const lines = [];
  for (const product of PRODUCTS) {
    const pageId = process.env[product.page];
    const pageToken = process.env[product.pageToken];
    const igId = process.env[product.ig];
    const igToken = process.env[product.igToken] || pageToken;
    if (pageId && pageToken) {
      const page = await get(`https://graph.facebook.com/${process.env.META_GRAPH_VERSION || "v25.0"}/${pageId}?fields=followers_count,fan_count&access_token=${encodeURIComponent(pageToken)}`);
      const activity = await metaActivity(pageId, pageToken, "facebook").catch(() => null);
      lines.push(`• ${product.id} Facebook: ${page.followers_count || page.fan_count || 0} followers${activity ? `, ${activity.reach} reach, ${activity.plays} plays, ${channelClicks("facebook", product.id)} tracked link clicks` : ""}`);
    }
    if (igId && igToken) {
      const instagram = await get(`https://graph.facebook.com/${process.env.META_GRAPH_VERSION || "v25.0"}/${igId}?fields=followers_count,media_count&access_token=${encodeURIComponent(igToken)}`);
      const activity = await metaActivity(igId, igToken, "instagram").catch(() => null);
      lines.push(`• ${product.id} Instagram: ${instagram.followers_count || 0} followers, ${instagram.media_count || 0} posts${activity ? `, ${activity.reach} reach, ${activity.plays} plays, ${channelClicks("instagram", product.id)} tracked link clicks` : ""}`);
    }
  }
  return lines;
});

const totalStars = productMetrics.reduce((sum, item) => sum + item.stars, 0);
const elapsed = Math.max(0, Math.min(90, Math.floor((Date.now() - new Date("2026-09-05T00:00:00Z")) / 86_400_000)));
const expected = 98 + Math.floor(102 * elapsed / 90);
sections.unshift(`**90-day goal**\n• ${totalStars}/200 combined stars; day ${elapsed}, linear checkpoint ${expected}\n• checkpoints: day 30 = 132 · day 60 = 166 · day 90 = 200`);

const artifacts = await actionArtifacts().catch(() => []);
await section("Publishing", async () => {
  const succeeded = artifacts.filter(({ name }) => name.startsWith("promo-") && !name.includes("failure")).length;
  const counts = new Map();
  for (const { name } of artifacts) {
    const match = name.match(/^promo-(auth-)?failure-([a-z]+)-/);
    if (match) counts.set(`${match[2]}${match[1] ? " auth" : ""}`, (counts.get(`${match[2]}${match[1] ? " auth" : ""}`) || 0) + 1);
  }
  return [`• ${succeeded} successful destination posts`, ...[...counts].map(([channel, count]) => `• ${channel}: ${count} failures`)];
});
const sourceClicks = Object.fromEntries(["hookecho", "stormdesk"].map((product) => [
  product,
  cta.filter((row) => row.target === `${product}-source` && SOCIAL_CHANNELS.has(row.placement)).reduce((sum, row) => sum + Number(row.clicks || 0), 0),
]));
const deltas = Object.fromEntries(productMetrics.map((item) => [item.id, item.starDelta]));
const sourcePosts = Object.fromEntries(["hookecho", "stormdesk"].map((product) => [
  product,
  artifacts.filter(({ name }) => name.includes(`-open-source-${product}-`) && !name.includes("failure")).length,
]));
const rates = Object.fromEntries(["hookecho", "stormdesk"].map((product) => [product, sourceClicks[product] / Math.max(1, sourcePosts[product])]));
const saturday = rates.hookecho === rates.stormdesk
  ? (deltas.stormdesk || 0) > (deltas.hookecho || 0) ? "stormdesk" : "hookecho"
  : rates.stormdesk > rates.hookecho ? "stormdesk" : "hookecho";
const pauses = promotionPauses(artifacts, cta);
writeOutput("saturday_product", saturday);
writeOutput("promotion_pauses", pauses);
const activePauses = new Set(artifacts.filter(({ name }) => name.startsWith("promotion-pauses-")).flatMap(({ name }) => name.split("-")));
const pausedChannels = pauses.split(",").map((entry) => entry.split("=")[0]).filter((channel) => channel && !activePauses.has(channel));
writeOutput("paused_channels", pausedChannels.join("-"));
const promotionDays = Math.floor((Date.now() - new Date("2026-09-08T00:00:00Z")) / 86_400_000);
const reportWeek = Math.floor((promotionDays + 1) / 7);
writeOutput("should_rebalance", String(reportWeek > 0 && reportWeek % 2 === 0));

const body = [
  "**HookEcho + StormDesk — weekly promotion report**",
  ...sections,
  failures.length ? `_optional sources that failed: ${failures.join("; ")}_` : "",
].filter(Boolean).join("\n\n").slice(0, 1990);

console.log(body);
if (!process.env.DISCORD_WEBHOOK_URL) {
  console.log("discord: skipped (DISCORD_WEBHOOK_URL unset)");
} else {
  await fetchText(process.env.DISCORD_WEBHOOK_URL, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ content: body }),
  });
  console.log("discord: posted");
}

async function ctaMetrics() {
  if (!process.env.CLOUDFLARE_API_TOKEN || !process.env.CLOUDFLARE_ACCOUNT_ID) return [];
  const sql = `SELECT blob1 AS target, blob2 AS placement, SUM(double1 * _sample_interval) AS clicks FROM hookecho_cta WHERE timestamp >= NOW() - INTERVAL '14' DAY GROUP BY target, placement ORDER BY clicks DESC`;
  const data = await get(`https://api.cloudflare.com/client/v4/accounts/${process.env.CLOUDFLARE_ACCOUNT_ID}/analytics_engine/sql`, {
    method: "POST",
    headers: { authorization: `Bearer ${process.env.CLOUDFLARE_API_TOKEN}`, "content-type": "text/plain" },
    body: sql,
  });
  return data.data || [];
}

async function googleToken(refreshToken) {
  const data = await get("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: process.env.GOOGLE_OAUTH_CLIENT_ID,
      client_secret: process.env.GOOGLE_OAUTH_CLIENT_SECRET,
      refresh_token: refreshToken,
      grant_type: "refresh_token",
    }),
  });
  return data.access_token;
}

function channelClicks(channel, product) {
  return Math.round(cta.filter((row) => row.placement === channel && (product === "stormdesk" ? row.target.startsWith("stormdesk") : !row.target.startsWith("stormdesk"))).reduce((sum, row) => sum + Number(row.clicks || 0), 0));
}

async function metaActivity(id, token, type) {
  const graph = `https://graph.facebook.com/${process.env.META_GRAPH_VERSION || "v25.0"}`;
  const edge = type === "facebook" ? "posts" : "media";
  const fields = type === "facebook" ? "id,created_time" : "id,timestamp,media_product_type";
  const media = await get(`${graph}/${id}/${edge}?fields=${fields}&since=${Math.floor(SINCE / 1000)}&limit=25&access_token=${encodeURIComponent(token)}`);
  const items = (media.data || []).filter((item) => type === "facebook" || item.media_product_type === "REELS");
  const totals = await Promise.all(items.map(async (item) => ({
    reach: await metaInsight(graph, item.id, token, type === "facebook" ? ["post_impressions_unique"] : ["reach"], type),
    plays: await metaInsight(graph, item.id, token, type === "facebook" ? ["post_video_views", "post_video_views_organic"] : ["views", "plays"], type),
  })));
  return totals.reduce((sum, item) => ({ reach: sum.reach + item.reach, plays: sum.plays + item.plays }), { reach: 0, plays: 0 });
}

async function metaInsight(graph, id, token, alternatives, type) {
  for (const metric of alternatives) {
    try {
      const period = type === "facebook" ? "&period=lifetime" : "";
      const result = await get(`${graph}/${id}/insights?metric=${metric}${period}&access_token=${encodeURIComponent(token)}`);
      const row = result.data?.[0];
      const value = row?.total_value?.value ?? row?.values?.at(-1)?.value ?? row?.value ?? 0;
      if (Number.isFinite(Number(value))) return Number(value);
    } catch {
      // Meta renames metrics across Graph versions; try the documented predecessor.
    }
  }
  return 0;
}

async function actionArtifacts() {
  if (!process.env.GITHUB_TOKEN || !process.env.GITHUB_REPOSITORY) return [];
  const all = [];
  for (let page = 1; page <= 5; page++) {
    const data = await get(`https://api.github.com/repos/${process.env.GITHUB_REPOSITORY}/actions/artifacts?per_page=100&page=${page}`, github());
    all.push(...(data.artifacts || []));
    if ((data.artifacts || []).length < 100) break;
  }
  return all.filter((artifact) => !artifact.expired && new Date(artifact.created_at).valueOf() > Date.now() - 14 * 86_400_000);
}

function writeOutput(name, value) {
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`);
}
