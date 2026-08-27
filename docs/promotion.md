# Releasing and promoting

How a release goes out, and what happens by hand afterwards. This doubles as the
release checklist — there isn't a separate one.

## Releasing

1. Bump `version` in `Cargo.toml`.
2. Write the version's section in [`CHANGELOG.md`](../CHANGELOG.md). **Not
   optional** — `release.yml` extracts it as the release body and fails the job
   if it is missing.
3. Commit, then `git tag vX.Y.Z && git push origin main --tags`.

Everything below then happens on its own:

| Workflow | Trigger | Does |
|---|---|---|
| `release.yml` | tag `v*`, and every push to main | builds AppImage / Windows / macOS / Android / container, attaches them, and uses the CHANGELOG section as the notes |
| `demo.yml` | push to main | rebuilds and deploys <https://app.hookecho.io/>, then smoke-tests it |
| `site.yml` | push to main touching `site/**` | builds and deploys the website, <https://hookecho.io/> |
| `announce.yml` | tag `v*` | posts to Bluesky, Mastodon, X and Discord, and attaches `announce-drafts.md` to the release |
| `digest.yml` | Mondays | mentions and metrics into Discord |

4. Download `announce-drafts.md` from the release and post it by hand, in wave
   order.

## Launch waves

One release, waves a few days apart. Do not do them all in one day — posting the
same thing in six places at once is the shape spam has.

**Wave 1 — technical. Skipped for the 2026-08 launch** by decision: no Show HN
and no r/rust launch post. The Rust-ecosystem *listings* below still apply.

**Wave 2 — domain.** r/tornado, r/weather, r/stormchasing, r/meteorology,
r/TropicalWeather (only with a named storm active), plus Stormtrack's Equipment
section, TalkWeather and chaser Discords. Lead
with the live demo, and pick a day with active weather so it shows something.
Lead with what it does for them — soundings, effective-layer parameters, archive
replay, chase packs — not with the language it is written in.

**Wave 3 — broad.** r/opensource, r/selfhosted (the container image qualifies),
r/linux. Lead with MIT, no accounts, no telemetry, no hosted service.

Rules that keep this from backfiring:

- Read the subreddit's self-promotion rule before posting. Some require a flair,
  some require prior participation, some ban it outright.
- One community per day, maximum.
- Answer every question, including the hostile ones, including "why not just use
  RadarScope".
- Never ask for stars.
- Keep the positioning honest: local-first, no telemetry, free. It is the actual
  differentiator against the paid Windows tools, and it stops being one the day
  it stops being true.

## One-time submissions

Do these once, not per release:

- [awesome-rust](https://github.com/rust-unofficial/awesome-rust) — Applications.
- [awesome-selfhosted](https://github.com/awesome-selfhosted/awesome-selfhosted) —
  the ghcr.io image is the qualifier.
- [AlternativeTo](https://alternativeto.net/) — listed against RadarScope,
  GR2Analyst and Supercell-Wx.
- [This Week in Rust](https://this-week-in-rust.org/) — "Call for participants"
  or the project spotlight.
- The r/rust "What's everyone working on this week" thread.

## Launch calendar

Two weeks, one community per day, weather communities on days with active
weather so the demo shows something:

| Day | Where | Lead with |
|---|---|---|
| 1 | r/opensource | MIT, no accounts, no telemetry |
| 2 | r/selfhosted | the ghcr.io image |
| 3 | r/linux | one binary, AppImage/Flatpak, wgpu |
| 4 | r/tornado | storm-replay clip |
| 5 | r/weather | storm-replay clip |
| 6 | r/stormchasing | archive replay, chase packs |
| 7 | r/meteorology | Level 2 all tilts, dual-pol |
| 8 | Stormtrack (Equipment) | free alternative to the paid Windows tools |
| 9 | TalkWeather | same, with the live demo link |
| 10 | Product Hunt (Tue–Thu) | tagline + maker's first comment |
| — | r/TropicalWeather | only with a named storm active, if rules allow |

Every post: demo link first, download second. Then stay at the keyboard.

## Sustained

- **Storm-event pages** within ~48 h of a major event: a `site/src/content/storms`
  entry plus a short blog post, shareable in that event's threads where on-topic.
- **One explainer post a month** on the site blog, on a title the existing posts
  do not already cover.
- **Weekly**: the r/rust "What's everyone working on" thread, and answering
  radar-app recommendation threads as they appear.

## Conventions

- **UTM tags** on links you paste by hand only: `?utm_source=reddit`,
  `?utm_source=hn`, and so on. The automated posts do not carry them — the
  channel is already known from the referrer.
- **Posting.** Reddit submissions go out through the Reddit API from the
  maintainer's own account (`scripts/announce/reddit.mjs`), one community per day
  — the cadence, not the mechanism, is what keeps it from reading as spam.
  **Replies are always written by hand**, and forums, Product Hunt and Discords
  have no posting API, so those stay copy-paste from the drafts. Reddit
  credentials live in a local gitignored env file: never committed, never a
  GitHub secret, because posting is a local action and not CI.
- **Analytics** is Cloudflare Web Analytics on the Pages project: server-side,
  no script and no code in the app, which is what keeps the no-telemetry claim
  true.
- **Secrets** the workflows expect, in repo Settings → Secrets → Actions:
  `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `BSKY_HANDLE`,
  `BSKY_APP_PASSWORD`, `MASTODON_URL`, `MASTODON_TOKEN`, `X_API_KEY`,
  `X_API_SECRET`, `X_ACCESS_TOKEN`, `X_ACCESS_SECRET`, `DISCORD_WEBHOOK_URL`.
  Any that are unset simply skip that channel.
- **Social preview image** in repo settings: it is the link card every channel
  renders, and the announce posts deliberately carry no uploaded media because
  of it. A crop of `docs/shots/hero.gif` does the job.
- **Assets** — `scripts/shots/shoot.sh` regenerates every screenshot and GIF in
  the README headlessly, so a stale asset is one command, not a screen-recording
  session.
