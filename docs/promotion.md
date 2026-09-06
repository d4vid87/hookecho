# Releasing and promoting

HookEcho owns the promotion automation for both HookEcho and WeatherDesk. It uses GitHub Actions,
the platform APIs and real product captures; there is no marketing service, paid media, LLM or
audience database.

The 90-day run starts on 2026-09-08. The recorded baseline is 98 combined GitHub stars (HookEcho
56, WeatherDesk 42), with checkpoints of 132 on day 30, 166 on day 60 and 200 on day 90. The
machine-readable baseline is [`scripts/announce/baseline.json`](../scripts/announce/baseline.json).

## Schedule

| UTC time | Campaign | Destinations |
|---|---|---|
| Tuesday 16:00 | HookEcho feature or radar education | Bluesky, Mastodon, HookEcho YouTube/Facebook/Instagram |
| Thursday 16:00 | WeatherDesk integration or whole-home use | Bluesky, Mastodon, WeatherDesk YouTube/Facebook/Instagram |
| Saturday 16:00 | Open-source, privacy, contributor or archive proof | Bluesky, Mastodon and the selected product accounts |
| Every 30 minutes | Qualifying official weather | Bluesky, Mastodon and HookEcho Facebook |
| Monday 14:00 | Metrics and opportunity report | Discord |
| Stable release | Relevant product release | Its owned accounts plus Discord |

The 13-week catalog is in [`scripts/announce/campaigns.mjs`](../scripts/announce/campaigns.mjs).
Saturday and stable-release copy contains the restrained GitHub-star request. Routine Tuesday and
Thursday posts and every safety post do not. Once the combined total reaches 200, only stable
releases retain the request.

Discord receives releases, weekly reports, failures and noteworthy mentions, not every scheduled
post. X stays disabled while its API requires paid credits. The system does not post or reply in
forums, Reddit, DMs or third-party communities; the Monday report surfaces opportunities for a
person to assess.

## Safety weather rules

The monitor reads the NWS active-alerts service and the three NHC RSS feeds. A post is eligible only
for an Actual/Extreme NWS alert, a Tornado Emergency or Flash Flood Emergency, or a recent named
tropical-cyclone item carrying a US watch or warning.

Copy is restricted to the official headline, affected area, issue/expiry times and official link.
It always says HookEcho is not an official warning source. It never contains a forecast
interpretation, casualty or damage estimate, or star request. The publisher allows at most one new
event in six hours and two per rolling day. A repeat event needs a changed official headline or
severity. Missing, expired or malformed input produces no public post; workflow failures are sent
to Discord.

Seven different successful monitor days are required before the first live-weather post. The
monitor runs while general publishing is disabled, so this observation period can happen during
account setup.

## Media

[`scripts/announce/render-social.sh`](../scripts/announce/render-social.sh) turns the selected real
HookEcho capture or WeatherDesk v4 demo-data screenshot into a 20-second, 1080×1920 H.264 video. It
burns in the campaign caption, product identity and `LIVE`, `ARCHIVE` or `DEMO DATA`, includes a
silent AAC track, emits `yuv420p`, and validates the result with `ffprobe`.

YouTube and Meta receive the bytes through their resumable upload APIs; no temporary public media
host is used. YouTube uploads remain disabled until the Google Cloud project passes its API audit.

## One-time authorization

Create separate HookEcho and WeatherDesk YouTube channels, Facebook Pages and Instagram
professional accounts. One Google Cloud project and one Meta developer app may authorize both
sets. Complete the provider reviews, then add these GitHub Actions secrets:

- Existing: `BSKY_HANDLE`, `BSKY_APP_PASSWORD`, `MASTODON_URL`, `MASTODON_TOKEN`,
  `DISCORD_WEBHOOK_URL`, `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_TOKEN`.
- GitHub metrics: optional `PROMOTION_GITHUB_TOKEN` with read access to traffic for both
  repositories. The workflow token remains the fallback.
- Google: `GOOGLE_OAUTH_CLIENT_ID`, `GOOGLE_OAUTH_CLIENT_SECRET`,
  `YOUTUBE_HOOKECHO_REFRESH_TOKEN`, `YOUTUBE_WEATHERDESK_REFRESH_TOKEN`.
- Meta: `META_HOOKECHO_PAGE_ID`, `META_HOOKECHO_PAGE_TOKEN`,
  `META_HOOKECHO_IG_USER_ID`, `META_HOOKECHO_IG_TOKEN`, and the four equivalent
  `META_WEATHERDESK_*` secrets.

Repository variables are the controls:

- `PROMOTION_ENABLED=false` is the master safety switch for scheduled and weather posts.
- `LIVE_WEATHER_ENABLED=true` permits weather posts after the seven-day observation gate.
- `YOUTUBE_PUBLIC_UPLOADS=false` keeps uploads private until the API audit is complete.
- `META_GRAPH_VERSION=v25.0` pins the reviewed Meta API version.
- `SATURDAY_PRODUCT=hookecho` is the fallback until the first two-week allocation is recorded.

Run Promotion manually with its default `dry_run=true` for each campaign kind before enabling the
master switch. A deliberate manual run with `dry_run=false` bypasses the master switch for test
accounts and keeps YouTube private until `YOUTUBE_PUBLIC_UPLOADS=true`.

## Publishing and recovery

[`scripts/announce/post.mjs`](../scripts/announce/post.mjs) is the one product-aware publisher.
Every publication has a deterministic product/content/platform ID. A successful destination writes
a 90-day Actions artifact, so a rerun skips it. Network throttles and server failures use bounded
exponential backoff. One provider failure does not fail other destinations.

Failure artifacts feed the Monday controller. Three authentication failures pause only that
channel. Four successful posts with no tracked click also pause it for two weeks.
Saturday's product is reassigned to the better repository-click rate every two weeks; Tuesday and
Thursday never move. Allocation and pause decisions are short-lived Actions artifacts, not an
external database.

To rehearse locally:

```sh
node --test scripts/announce/promotion.test.mjs
node scripts/announce/post.mjs prepare-scheduled /tmp/campaign.json 2026-09-08T16:00:00Z hookecho
scripts/announce/render-social.sh /tmp/campaign.json /tmp/social.mp4
DRY_RUN=true node scripts/announce/post.mjs publish /tmp/campaign.json bluesky /tmp/social.mp4
```

## Releases

For HookEcho, bump `Cargo.toml`, add the matching `CHANGELOG.md` section, commit, then push a stable
`vX.Y.Z` tag. `release.yml` creates the packages; `announce.yml` waits for that release and uses the
shared publisher. Prerelease tags containing `-` are not announced automatically.

WeatherDesk stable releases are detected hourly from its latest GitHub release, so its application
repository needs no duplicated platform credentials or publishing code.

## Measurement

Tracked links use the site's allowlisted `/go/.../{placement}` redirects. Cloudflare Analytics
Engine stores only target, channel placement, count and its automatic timestamp—never an IP,
location, referrer, user-agent, cookie or identifier.

The Monday report includes stars and seven-day change, checkpoint progress, tracked source/product
clicks, available GitHub traffic and downloads, YouTube views/watch duration/shares/subscribers,
aggregate Meta reach/plays/tracked-link clicks, failures and public GitHub/HN/Bluesky mentions.
Missing optional data is reported without blocking the rest.

The website sitemap, RSS, press kit, glossary, comparison pages and storm archive remain the long-
term discovery surfaces. Do not mass-generate search pages or add a newsletter. Human community
participation stays human: read each venue's rules, identify yourself, and never automate replies.
