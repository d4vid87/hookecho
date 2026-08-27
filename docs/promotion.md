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

**Wave 2 — domain.** Stormtrack's Equipment section, TalkWeather and chaser
Discords. Lead
with the live demo, and pick a day with active weather so it shows something.
Lead with what it does for them — soundings, effective-layer parameters, archive
replay, chase packs — not with the language it is written in.

**Wave 3 — broad.** Product Hunt, DevHunt, Uneed. Lead with MIT, no accounts, no telemetry, no hosted service.

Rules that keep this from backfiring:

- Read the venue's self-promotion rule before posting. Some require prior
  participation, some ban it outright.
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
  Gated on `stars > 50 | crates.io downloads > 2000`; at 48 stars on 2026-08-27,
  so this waits. They judge on the metric and nothing else.
- [awesome-selfhosted](https://github.com/awesome-selfhosted/awesome-selfhosted) —
  the ghcr.io image is the qualifier, but their checklist requires the first
  release to be **more than 4 months old**: first release was 2026-07-29, so the
  earliest valid submission is ~2026-11-29. Entries go to the
  `awesome-selfhosted-data` repo as `software/<slug>.yml`, not the README.
- [AlternativeTo](https://alternativeto.net/) — listed against RadarScope,
  GR2Analyst and Supercell-Wx.
- [This Week in Rust](https://this-week-in-rust.org/) — Project/Tooling Updates
  no longer takes PRs (editors pull those from r/rust). Call for Participation
  does: submitted as
  [#8671](https://github.com/rust-lang/this-week-in-rust/pull/8671) against the
  2026-09-02 draft. Their guidelines want the linked issue to state a difficulty
  and link CONTRIBUTING.md — issue #12 was edited to do both.

## Launch calendar

One venue per day, weather venues on days with active weather so the demo shows
something. **No Reddit** — dropped for this launch, so the sub rows are gone
rather than left as drafts:

| Day | Where | Lead with |
|---|---|---|
| 1 | Stormtrack (Equipment) | free alternative to the paid Windows tools |
| 2 | TalkWeather | same, with the live demo link |
| 3 | Chaser Discords | storm-replay clip |
| 4 | Product Hunt (Tue–Thu) | tagline + maker's first comment |
| 5 | DevHunt, Uneed | MIT, no accounts, no telemetry |
| 6 | Bluesky / Mastodon / X | storm-replay clip, demo link |

Every post: demo link first, download second. Then stay at the keyboard.

## Sustained

- **Storm-event pages** within ~48 h of a major event: a `site/src/content/storms`
  entry plus a short blog post, shareable in that event's threads where on-topic.
- **One explainer post a month** on the site blog, on a title the existing posts
  do not already cover.
- **Weekly**: answering radar-app recommendation threads as they appear, on the
  venues above.

## Conventions

- **UTM tags** on links you paste by hand only: `?utm_source=stormtrack`,
  `?utm_source=producthunt`, and so on. The automated posts do not carry them — the
  channel is already known from the referrer.
- **Posting.** Reddit is out for the 2026-08 launch — no API rig, no submissions.
  Everything else has no posting API either, so forums, Product Hunt, the
  directories and the Discords are copy-paste from the drafts, and **replies are
  always written by hand**. The automated channels stay what `announce.yml`
  already does: Bluesky, Mastodon, X, Discord, on a tag.
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
