// Service worker: app-shell precache, nothing else.
//
// web/sw.js is generated from this file by scripts/web/build.sh, which substitutes SHELL with the
// content-hashed asset list of the build that produced it. That list is the version: a new build
// is a new script body, which is what makes the browser install a new worker at all.
//
// ponytail: precache only. Radar, tiles and everything that goes through /proxy stay on the
// network — a cached radar frame is a wrong radar frame, and the wasm is the only thing here big
// enough for caching to be worth a stale-content risk.

// Replaced at build time. Every entry is either a content-hashed bundle file or the page
// itself, which is revalidated on every navigation.
const SHELL = __SHELL__;

// Named for the shell it holds, so installing a new worker never collides with the old cache and
// the activate step can delete every cache that is not this one.
const CACHE = "shell-" + __VERSION__;

self.addEventListener("install", (e) => {
  // The new worker takes over on the next navigation, not this one — the running page is already
  // wired to the glue it booted with, and swapping the wasm underneath it would break it.
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  // /proxy and /geo.json are per-request answers: the proxy streams live data, and /geo.json is a
  // different answer for every visitor. Neither is ever served from here.
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
