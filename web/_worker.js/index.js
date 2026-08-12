// The CORS proxy as a Cloudflare Pages Worker. The checks and the allowlist live in
// proxy-core.js, shared with the Netlify edge function; this file is only the Pages wiring —
// static assets through `env.ASSETS`, and Cloudflare's own cache hints on the upstream fetch.
//
// This is a `_worker.js/` directory rather than a single file on purpose: Pages only bundles
// imported modules in the directory form.

import { cacheSeconds, handleProxy } from "./proxy-core.js";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!url.pathname.startsWith("/proxy/")) return env.ASSETS.fetch(request);
    return handleProxy(request, {
      fetchInit: (host) => ({ cf: { cacheTtl: cacheSeconds(host), cacheEverything: true } }),
    });
  },
};
