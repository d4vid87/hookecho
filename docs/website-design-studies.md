# Ten HookEcho website directions

Open `http://127.0.0.1:4347/designs/` after running `npm run dev --prefix site -- --host 127.0.0.1 --port 4347` from this checkout.

| Design | Palette | Navigation / composition | Taste-vault references |
| --- | --- | --- | --- |
| Cirrus | White, ice blue, cobalt | Floating pills; clear conditions card | Yr, Apple |
| Meridian | Navy, slate, mint | Left rail; map-led workspace | Windy, Linear |
| Sunday | Cream, terracotta | Centered editorial masthead; paired forecast cards | BBC, Opal |
| Ion | Black, acid lime | Numbered control strip; instrument grid | Teenage Engineering, Nothing |
| Prism | Lavender, violet | Grouped Explore menu; bento dashboard | Stripe, Linear |
| Tidal | Sea glass, teal | Bottom dock; panoramic radar | RainViewer, Apple |
| Ember | Charcoal, warm orange | Right rail; forecast-first planning | Porsche, Warm Ember vault palette |
| Alpine | Ivory, forest green | Attached tabs; field-guide cards | Met Office, Yr |
| Nocturne | Midnight, ice blue | Expandable menu; immersive radar | Porsche, Ventusky |
| Dispatch | White, slate, red | Two-level masthead; compact forecast ledger | BBC, Time and Date |

All variants render `HomeContent.astro`, shared with the current homepage. WeatherOverview, grouped dates/day-night details, place search, saved locations, units, geolocation, NWS alerts, the actual radar iframe, Analyst demonstration, feature captures, and download links remain shared. The local Vite proxy forwards `/api/place` to the existing published service. Documentation and other secondary pages keep the existing site design.

The gallery's JPEG previews deliberately use consistent sample forecast data and recorded radar. The actual design pages use live services. Nocturne is the selected production homepage. Its shared layout removes the review navigation and restores homepage SEO metadata and optional analytics. The other studies remain an unlinked archive. Preview pages are noindexed and excluded from the sitemap and Pagefind search.

Validation: `npm test --prefix site`, `npm run build --prefix site`, `npm run test:site --prefix site`, and `node site/scripts/designs-smoke.mjs`. The latter requires the declared Playwright dev dependency and a browser; `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH=/usr/bin/chromium` selects a system Chromium. Pass `--capture` to refresh gallery thumbnails. The existing forecast and Analyst smoke checks cover the shared homepage behavior.
