import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  campaignForDate,
  composeText,
  eligibleNhcItems,
  eligibleNwsAlert,
  isPaused,
  pickWeatherCampaign,
  promotionPauses,
  releaseCampaign,
  trackedUrl,
  weatherCapAllows,
} from "./campaigns.mjs";
import { channelSecrets, linkFacets, markerName, publishFile, retryingFetch } from "./post.mjs";

test("the 13-week calendar keeps fixed product slots and Saturday star asks", () => {
  const tuesday = campaignForDate("2026-09-08T16:00:00Z");
  const thursday = campaignForDate("2026-09-10T16:00:00Z");
  const saturday = campaignForDate("2026-09-12T16:00:00Z", "weatherdesk");
  assert.equal(tuesday.product, "hookecho");
  assert.equal(thursday.product, "weatherdesk");
  assert.equal(saturday.product, "weatherdesk");
  assert.equal(tuesday.askForStar, false);
  assert.equal(saturday.askForStar, true);
  assert.equal(campaignForDate("2026-12-08T16:00:00Z"), null);
});

test("release copy, channel limits, redirects, facets, secrets and pauses are bounded", () => {
  const hook = releaseCampaign("hookecho", "v1.2.3", "## 1.2.3 — Test\n- First thing — detail\n- Second thing");
  const desk = releaseCampaign("weatherdesk", "v4.1.0", "## [4.1.0]\n- New dashboard");
  hook.combinedStars = 98;
  desk.combinedStars = 200;
  assert.match(hook.body, /First thing/);
  assert.equal(markerName(hook, "youtube"), markerName(hook, "youtube"));
  assert.match(trackedUrl(hook, "youtube"), /download\/youtube$/);
  assert.match(trackedUrl(desk, "facebook"), /weatherdesk-release\/facebook$/);
  assert.ok(composeText(hook, "bluesky").length <= 300);
  assert.match(composeText(desk, "mastodon"), /give it a star/);
  hook.body = "Long release detail ".repeat(80);
  assert.match(composeText(hook, "bluesky"), /give it a star/);
  for (const channel of ["bluesky", "mastodon", "discord", "youtube", "facebook", "instagram"]) {
    assert.ok(composeText(hook, channel).length <= (channel === "bluesky" ? 300 : channel === "mastodon" ? 500 : 1900));
  }
  const completedSaturday = campaignForDate("2026-09-12T16:00:00Z");
  completedSaturday.combinedStars = 200;
  assert.doesNotMatch(composeText(completedSaturday, "mastodon"), /give it a star/);
  assert.equal(linkFacets("é https://example.com")[0].index.byteStart, 3);
  assert.match(markerName(hook, "youtube"), /^promo-release-hookecho-v1.2.3-youtube$/);
  assert.ok(channelSecrets("youtube", "hookecho", {
    GOOGLE_OAUTH_CLIENT_ID: "a",
    GOOGLE_OAUTH_CLIENT_SECRET: "b",
    YOUTUBE_HOOKECHO_REFRESH_TOKEN: "c",
  }));
  assert.equal(isPaused("facebook", "facebook=2099-01-01T00:00:00Z", new Date("2026-01-01")), true);
  const artifacts = [1, 2, 3].map((i) => ({ name: `promo-auth-failure-facebook-${i}`, created_at: `2026-09-0${i}T00:00:00Z` }));
  assert.match(promotionPauses(artifacts, [], new Date("2026-09-05T00:00:00Z")), /facebook=/);
  artifacts.push({ name: "promo-scheduled-test-facebook", created_at: "2026-09-04T00:00:00Z" });
  assert.doesNotMatch(promotionPauses(artifacts, [{ placement: "facebook", clicks: 1 }], new Date("2026-09-05T00:00:00Z")), /facebook=/);
});

test("official weather gates reject stale or invented risk and allow only bounded updates", () => {
  const now = new Date("2026-09-05T18:00:00Z");
  const base = {
    id: "https://api.weather.gov/alerts/one",
    properties: {
      status: "Actual",
      severity: "Extreme",
      headline: "Extreme Wind Warning issued September 5",
      areaDesc: "Example County",
      sent: "2026-09-05T17:00:00Z",
      expires: "2026-09-05T20:00:00Z",
    },
  };
  const first = eligibleNwsAlert(base, now);
  assert.ok(first);
  assert.equal(first.askForStar, false);
  assert.match(composeText(first, "bluesky"), /not an official warning source/i);
  assert.equal(eligibleNwsAlert({ ...base, properties: { ...base.properties, severity: "Severe" } }, now), null);
  const emergency = eligibleNwsAlert({
    ...base,
    properties: { ...base.properties, severity: "Severe", headline: "Tornado Warning", description: "TORNADO EMERGENCY" },
  }, now);
  assert.ok(emergency);
  assert.notEqual(first.id, eligibleNwsAlert({ ...base, properties: { ...base.properties, headline: "Updated warning" } }, now).id);
  const vtec = "/O.NEW.KOUN.TO.W.0001.260905T1700Z-260905T2000Z/";
  const continued = "/O.CON.KOUN.TO.W.0001.260905T1700Z-260905T2000Z/";
  const vtecFirst = eligibleNwsAlert({ ...base, properties: { ...base.properties, parameters: { VTEC: [vtec] } } }, now);
  const sameUpdate = eligibleNwsAlert({ ...base, id: "https://api.weather.gov/alerts/two", properties: { ...base.properties, parameters: { VTEC: [continued] } } }, now);
  assert.equal(vtecFirst.id, sameUpdate.id);
  assert.equal(eligibleNwsAlert({ ...base, properties: { ...base.properties, expires: "2026-09-05T17:30:00Z" } }, now), null);

  const rss = `<rss><channel><item><title>Hurricane Example warning for Florida</title><description>Hurricane Warning for the United States coast</description><link>https://www.nhc.noaa.gov/text/MIATCPAT1.shtml</link><guid>x1</guid><pubDate>Sat, 05 Sep 2026 17:30:00 GMT</pubDate></item></channel></rss>`;
  assert.equal(eligibleNhcItems(rss, now).length, 1);
  assert.deepEqual(eligibleNhcItems("not xml", now), []);
  assert.equal(eligibleNhcItems(rss.replace("Florida", "Bermuda").replace("United States", "Bermuda"), now).length, 0);
  assert.equal(pickWeatherCampaign([base], [rss], new Set([first.id]), now).kind, "weather");
  assert.equal(weatherCapAllows([{ id: "a", createdAt: "2026-09-05T13:00:00Z" }], now), false);
  assert.equal(weatherCapAllows([{ id: "a", createdAt: "2026-09-04T13:00:00Z" }], now), true);
  assert.equal(weatherCapAllows([{ id: "a", createdAt: "2026-09-05T08:00:00Z" }, { id: "b", createdAt: "2026-09-05T11:00:00Z" }], now), false);
});

test("an existing success marker makes a retry a no-op", async () => {
  const dir = mkdtempSync(join(tmpdir(), "promotion-idempotency-"));
  const path = join(dir, "campaign.json");
  const campaign = { id: "scheduled-existing", kind: "scheduled", product: "hookecho", title: "Test", body: "Body", askForStar: false, combinedStars: 98 };
  writeFileSync(path, JSON.stringify(campaign));
  const before = { GITHUB_TOKEN: process.env.GITHUB_TOKEN, GITHUB_REPOSITORY: process.env.GITHUB_REPOSITORY, BSKY_HANDLE: process.env.BSKY_HANDLE, BSKY_APP_PASSWORD: process.env.BSKY_APP_PASSWORD };
  Object.assign(process.env, { GITHUB_TOKEN: "token", GITHUB_REPOSITORY: "d4vid87/hookecho", BSKY_HANDLE: "test", BSKY_APP_PASSWORD: "test" });
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    assert.match(String(url), /actions\/artifacts/);
    return Response.json({ artifacts: [{ name: markerName(campaign, "bluesky"), expired: false }] });
  };
  try {
    assert.equal(await publishFile(path, "bluesky"), false);
  } finally {
    globalThis.fetch = originalFetch;
    for (const [key, value] of Object.entries(before)) value === undefined ? delete process.env[key] : process.env[key] = value;
    rmSync(dir, { recursive: true, force: true });
  }
});

test("retrying fetch retries only transient failures", async () => {
  let calls = 0;
  const response = await retryingFetch("https://example.com", {}, async () => {
    calls++;
    return new Response(calls < 3 ? "later" : "ok", { status: calls < 3 ? 500 : 200 });
  }, async () => {});
  assert.equal(response.status, 200);
  assert.equal(calls, 3);
});

test("every provider builds a valid request without contacting a service", async () => {
  const dir = mkdtempSync(join(tmpdir(), "promotion-test-"));
  const campaignPath = join(dir, "campaign.json");
  const media = join(dir, "clip.mp4");
  writeFileSync(media, "video");
  writeFileSync(campaignPath, JSON.stringify({
    id: "scheduled-test",
    kind: "scheduled",
    product: "hookecho",
    title: "A real test",
    body: "Product copy",
    askForStar: false,
    combinedStars: 98,
  }));

  const keys = [
    "PROMOTION_ENABLED", "YOUTUBE_PUBLIC_UPLOADS", "BSKY_HANDLE", "BSKY_APP_PASSWORD",
    "MASTODON_URL", "MASTODON_TOKEN", "DISCORD_WEBHOOK_URL", "GOOGLE_OAUTH_CLIENT_ID",
    "GOOGLE_OAUTH_CLIENT_SECRET", "YOUTUBE_HOOKECHO_REFRESH_TOKEN", "META_HOOKECHO_PAGE_ID",
    "META_HOOKECHO_PAGE_TOKEN", "META_HOOKECHO_IG_USER_ID", "META_HOOKECHO_IG_TOKEN",
  ];
  const before = Object.fromEntries(keys.map((key) => [key, process.env[key]]));
  Object.assign(process.env, {
    PROMOTION_ENABLED: "true",
    YOUTUBE_PUBLIC_UPLOADS: "true",
    BSKY_HANDLE: "bot.test",
    BSKY_APP_PASSWORD: "secret",
    MASTODON_URL: "https://social.example",
    MASTODON_TOKEN: "secret",
    DISCORD_WEBHOOK_URL: "https://discord.example/hook",
    GOOGLE_OAUTH_CLIENT_ID: "id",
    GOOGLE_OAUTH_CLIENT_SECRET: "secret",
    YOUTUBE_HOOKECHO_REFRESH_TOKEN: "refresh",
    META_HOOKECHO_PAGE_ID: "page",
    META_HOOKECHO_PAGE_TOKEN: "page-token",
    META_HOOKECHO_IG_USER_ID: "ig",
    META_HOOKECHO_IG_TOKEN: "ig-token",
  });

  const originalFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    calls.push({ url: String(url), init });
    if (String(url).includes("createSession")) return Response.json({ accessJwt: "jwt", did: "did:test:one" });
    if (String(url).includes("oauth2.googleapis.com")) return Response.json({ access_token: "google" });
    if (String(url).includes("youtube") && init.method === "POST") return new Response("", { status: 200, headers: { location: "https://upload.example/youtube" } });
    if (String(url).includes("video_reels") && String(init.body).includes("upload_phase=start")) return Response.json({ video_id: "video", upload_url: "https://upload.example/facebook" });
    if (String(url).endsWith("/media")) return Response.json({ id: "container", uri: "https://upload.example/instagram" });
    if (String(url).includes("container?fields=status_code")) return Response.json({ status_code: "FINISHED" });
    return new Response(String(url).includes("createRecord") || String(url).includes("statuses") || String(url).includes("discord") || String(url).includes("media_publish") || String(url).includes("video_reels") ? "{}" : "{}", { status: 200 });
  };

  try {
    for (const channel of ["bluesky", "mastodon", "discord", "youtube", "facebook", "instagram"]) {
      await publishFile(campaignPath, channel, media);
    }
    assert.ok(calls.some((call) => call.url.includes("createRecord")));
    assert.ok(calls.some((call) => call.url.includes("youtube/v3/videos")));
    assert.ok(calls.some((call) => call.url.includes("video_reels")));
    assert.ok(calls.some((call) => call.url.includes("media_publish")));
    const bluesky = calls.find((call) => call.url.includes("createRecord"));
    assert.equal(JSON.parse(bluesky.init.body).record.$type, "app.bsky.feed.post");
  } finally {
    globalThis.fetch = originalFetch;
    for (const key of keys) before[key] === undefined ? delete process.env[key] : process.env[key] = before[key];
    rmSync(dir, { recursive: true, force: true });
  }
});
