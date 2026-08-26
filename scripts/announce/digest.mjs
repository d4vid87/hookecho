// Weekly digest: who mentioned the project, and what the numbers did. One Discord message.
//
//   DISCORD_WEBHOOK_URL=… GITHUB_TOKEN=… node scripts/announce/digest.mjs
//
// The point is a reply queue, not a dashboard: a mention nobody answers within a day is a mention
// wasted. Every source is optional — Reddit rate-limits unauthenticated search often enough that
// one dead source must not cost the whole digest, so each is caught and named instead.
//
// ponytail: no stored state. The window is 8 days against a weekly cron, so clock skew produces a
// repeat rather than a gap.
const REPO = "d4vid87/hookecho";
const QUERY = "hookecho OR \"HookEcho\"";
const SINCE = Date.now() - 8 * 24 * 3600 * 1000;
const UA = "hookecho-digest (github.com/d4vid87/hookecho)";

const sections = [];
const failures = [];

async function section(name, fn) {
  try {
    const lines = await fn();
    if (lines.length) sections.push(`**${name}**\n${lines.join("\n")}`);
  } catch (e) {
    failures.push(`${name}: ${e.message}`);
  }
}

const get = async (url, headers = {}) => {
  const resp = await fetch(url, { headers: { "user-agent": UA, ...headers } });
  if (!resp.ok) throw new Error(`${resp.status}`);
  return resp.json();
};

await section("Hacker News", async () => {
  const data = await get(
    `https://hn.algolia.com/api/v1/search_by_date?query=${encodeURIComponent("hookecho")}&numericFilters=created_at_i>${Math.floor(SINCE / 1000)}`,
  );
  return data.hits.slice(0, 5).map((h) => `• ${h.title || h.story_title} — https://news.ycombinator.com/item?id=${h.objectID}`);
});

await section("Reddit", async () => {
  const data = await get(
    `https://www.reddit.com/search.json?q=${encodeURIComponent(QUERY)}&sort=new&limit=25`,
  );
  return data.data.children
    .filter((c) => c.data.created_utc * 1000 > SINCE)
    .slice(0, 5)
    .map((c) => `• r/${c.data.subreddit}: ${c.data.title} — https://reddit.com${c.data.permalink}`);
});

await section("Bluesky", async () => {
  const data = await get(
    `https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts?q=${encodeURIComponent("hookecho")}&limit=25`,
  );
  return (data.posts || [])
    .filter((p) => new Date(p.indexedAt).getTime() > SINCE)
    .slice(0, 5)
    .map((p) => `• @${p.author.handle}: ${(p.record.text || "").replace(/\s+/g, " ").slice(0, 120)}`);
});

await section("GitHub", async () => {
  // Authenticated: `demote-old` in release.yml drafts old releases, and drafts are invisible to an
  // anonymous call — so historical download counts would silently vanish without the token.
  const auth = process.env.GITHUB_TOKEN ? { authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {};
  const repo = await get(`https://api.github.com/repos/${REPO}`, auth);
  const releases = await get(`https://api.github.com/repos/${REPO}/releases?per_page=100`, auth);
  const downloads = releases
    .flatMap((r) => r.assets)
    .reduce((sum, a) => sum + a.download_count, 0);
  return [
    `• ${repo.stargazers_count} stars, ${repo.forks_count} forks, ${repo.open_issues_count} open issues`,
    `• ${downloads} release-asset downloads all time`,
  ];
});

const body = [
  `**HookEcho — last 8 days**`,
  ...sections,
  failures.length ? `_sources that failed: ${failures.join("; ")}_` : "",
]
  .filter(Boolean)
  .join("\n\n")
  .slice(0, 1990);

console.log(body);

const hook = process.env.DISCORD_WEBHOOK_URL;
if (!hook) {
  console.log("discord: skipped (DISCORD_WEBHOOK_URL unset)");
} else {
  const resp = await fetch(hook, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ content: body }),
  });
  if (!resp.ok) throw new Error(`discord ${resp.status}: ${await resp.text()}`);
  console.log("discord: posted");
}
