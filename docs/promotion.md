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
| `demo.yml` | push to main | rebuilds and deploys <https://hookecho.pages.dev/>, then smoke-tests it |
| `announce.yml` | tag `v*` | posts to Bluesky, Mastodon, X and Discord, and attaches `announce-drafts.md` to the release |
| `digest.yml` | Mondays | mentions and metrics into Discord |

4. Download `announce-drafts.md` from the release and post it by hand, in wave
   order.

## Launch waves

One release, three waves, a few days apart. Do not do them all in one day —
posting the same thing in six places at once is the shape spam has.

**Wave 1 — technical.** r/rust and Show HN, US weekday morning (Tue–Thu, ~8am
ET). Lead with how it is built: wgpu pipeline, Level 2 decode, one codebase on
desktop, Android and wasm. Be at a keyboard for the next 6–12 hours; the replies
are the post.

**Wave 2 — domain.** r/meteorology, r/stormchasing, and chaser Discords. Lead
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

## Conventions

- **UTM tags** on links you paste by hand only: `?utm_source=reddit`,
  `?utm_source=hn`, and so on. The automated posts do not carry them — the
  channel is already known from the referrer.
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
