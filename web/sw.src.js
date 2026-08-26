// Service worker: app-shell precache, nothing else.
//
// web/sw.js is generated from this file by scripts/web/build.sh, which substitutes SHELL with the
// content-hashed asset list of the build that produced it. That list is the version: a new build
// is a new script body, which is what makes the browser install a new worker at all.
//
// Basemap tiles get a second, persistent cache. They are the one thing the app fetches over and
// over that never changes — a road at z9 is the same road next week — and without a filesystem
// behind `paths::cache_dir()` the web build otherwise re-downloads the entire visible map on
// every cold start.
//
// ponytail: radar, satellite and everything through /proxy stay on the network. A cached radar
// frame is a wrong radar frame, and the win here is geography, which is static by definition.

// Replaced at build time. Every entry is either a content-hashed bundle file or the page
// itself, which is revalidated on every navigation.
const SHELL = __SHELL__;

// Named for the shell it holds, so installing a new worker never collides with the old cache and
// the activate step can delete every cache that is not this one.
const CACHE = "shell-" + __VERSION__;

// Survives deploys — the tiles in it are not versioned by our build, and throwing them away
// because the wasm changed would be pure waste.
const TILES = "tiles-v1";

// Roughly a few hundred MB of raster at worst, which is well inside a normal origin quota and
// small enough that trimming stays cheap. Cache.keys() is insertion-ordered, so the oldest
// entries are the first ones — evicting from the front is FIFO with no bookkeeping to store.
const TILE_MAX = 1500;
const TILE_TRIM = 300;

// Is this an XYZ tile (or the glyphs and sprites a vector basemap needs to draw one)? Matched on
// URL shape rather than a host list: the app ships a dozen basemaps and lets the user paste their
// own template, most of them arrive rewritten as `/proxy/<host>/…` rather than cross-origin, and a
// host allowlist here would silently fall out of step with tiles.rs and proxy-core.js both.
//
// Shape is also what keeps radar out. A volume through the same proxy is
// `/proxy/…/KTLX/KTLX20260825_…`, which has no tile coordinate and no tile extension, so it never
// matches — the rule that keeps live data on the network is structural, not a host check.
//
// Keyed hosts (Mapbox, MapTiler) carry the user's key in the query, which becomes part of the
// cache key. That is the same place the browser's own HTTP cache already keeps it — origin-private
// storage on the user's machine — and nothing is ever sent anywhere new.
function isTile(url) {
  const p = url.pathname;
  return (
    /\/\d+\/\d+\/\d+(@\dx)?(\.[a-z0-9]+)?$/.test(p) ||
    /\.(pbf|mvt)$/.test(p) ||
    /\/fonts?\//.test(p) ||
    /sprite(@\dx)?\.(png|json)$/.test(p)
  );
}

// Drop the oldest entries once the cache has grown past its cap. Fire-and-forget: a trim that
// loses a race just runs again on the next tile.
async function trim() {
  const c = await caches.open(TILES);
  const keys = await c.keys();
  if (keys.length <= TILE_MAX) return;
  await Promise.all(keys.slice(0, TILE_TRIM).map((k) => c.delete(k)));
}

self.addEventListener("install", (e) => {
  // The new worker takes over on the next navigation, not this one — the running page is already
  // wired to the glue it booted with, and swapping the wasm underneath it would break it.
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys.filter((k) => k !== CACHE && k !== TILES).map((k) => caches.delete(k)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);

  if (isTile(url)) {
    // Cache-first, no revalidation: these URLs address fixed content (a tile coordinate, a glyph
    // range, a satellite timestamp), so a hit is the right answer and a conditional request would
    // only spend a round trip to be told so.
    e.respondWith(
      caches.match(req).then(
        (hit) =>
          hit ||
          fetch(req).then((res) => {
            // Opaque responses (a tile host with no CORS headers) are cacheable and replayable,
            // which is all the renderer needs. An error is not: caching a 404 tile would make a
            // transient outage permanent.
            if (res.ok || res.type === "opaque") {
              const copy = res.clone();
              // Not waitUntil: by the time this runs the response has usually already been
              // handed to the page, and a settled event rejects it. The worker stays alive for
              // the fetch anyway, and a store that loses the race costs one re-download.
              caches
                .open(TILES)
                .then((c) => c.put(req, copy))
                .then(trim)
                .catch(() => {});
            }
            return res;
          }),
      ),
    );
    return;
  }
  // Anything else off-origin is live data going straight to its host — let the browser have it.
  if (url.origin !== self.location.origin) return;
  // What is left under /proxy is live data too, and /geo.json is a different answer for every
  // visitor. Neither is ever served from here.
  if (url.pathname.startsWith("/proxy/") || url.pathname === "/geo.json") return;

  // A navigation asks for index.html, which names the hashed assets — serving a stale one would
  // point at a deploy that is already gone. Network first, cache only as the offline answer.
  if (req.mode === "navigate") {
    e.respondWith(
      fetch(req)
        .then((res) => {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put("/", copy));
          return res;
        })
        .catch(() => caches.match("/").then((r) => r || Response.error())),
    );
    return;
  }

  // Everything else in the shell is content-hashed, so a hit is by definition the right bytes.
  e.respondWith(caches.match(req).then((r) => r || fetch(req)));
});
