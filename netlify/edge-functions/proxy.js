// The same CORS proxy, as a Netlify edge function (Deno). Everything that matters — allowlist,
// GET-only, size cap, content-type narrowing — is in web/_worker.js/proxy-core.js, shared with the
// Cloudflare Worker. Deno has no `cf` fetch option, so the cache hint is a response header
// instead: `netlify-cdn-cache-control` is what Netlify's CDN reads, and `cache-control` is what
// the browser reads.

import { cacheSeconds, handleProxy } from "../../web/_worker.js/proxy-core.js";

export default async (request) =>
  handleProxy(request, {
    extraHeaders: (host, search) => {
      const ttl = cacheSeconds(host, search);
      return {
        "cache-control": `public, max-age=${ttl}`,
        "netlify-cdn-cache-control": `public, s-maxage=${ttl}, durable`,
      };
    },
  });

export const config = { path: "/proxy/*" };
