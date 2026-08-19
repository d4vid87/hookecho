// The CORS proxy as a Cloudflare Pages Worker. The checks and the allowlist live in
// proxy-core.js; this file is only the Pages wiring —
// static assets through `env.ASSETS`, and Cloudflare's own cache hints on the upstream fetch.
//
// This is a `_worker.js/` directory rather than a single file on purpose: Pages only bundles
// imported modules in the directory form.

import { cacheSeconds, handleProxy } from "./proxy-core.js";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    // The visitor's rough position, straight off the edge, so the app can open on the nearest
    // radar. Never cached — it is a
    // different answer for every visitor.
    if (url.pathname === "/geo.json") {
      const { latitude, longitude } = request.cf ?? {};
      const body = latitude && longitude ? JSON.stringify([+longitude, +latitude]) : "null";
      return new Response(body, {
        headers: { "content-type": "application/json", "cache-control": "private, no-store" },
      });
    }
    if (!url.pathname.startsWith("/proxy/")) return env.ASSETS.fetch(request);
    return handleProxy(request, {
      fetchInit: (host, search) => ({ cf: { cacheTtl: cacheSeconds(host, search), cacheEverything: true } }),
    });
  },
};
