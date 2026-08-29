// `node --test web/_worker.js/proxy-core.test.mjs`
//
// The proxy is the one piece of this repo with no Rust test behind it, and the parts worth
// pinning are pure: which host is refused, which TTL a URL gets, and when a conditional request
// is allowed to become a 304. Upstream is a stub — nothing here touches the network.

import { test } from "node:test";
import assert from "node:assert/strict";
import { cacheSeconds, handleProxy, notModified } from "./proxy-core.js";

const upstream = (headers = {}, body = "hello") =>
  new Response(body, { headers: { "content-type": "application/json", ...headers } });

// Stand in for the platform `fetch` handleProxy calls, remembering what it was asked for.
function stubFetch(response) {
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return response;
  };
  return calls;
}

const ask = (path, headers = {}) =>
  new Request(`https://example.test/proxy/${path}`, { headers });

const proxy = (request) =>
  handleProxy(request, {
    extraHeaders: (host, search) => ({ "cache-control": `public, max-age=${cacheSeconds(host, search)}` }),
  });

test("a proxied response carries the edge TTL and the upstream validators", async () => {
  stubFetch(upstream({ etag: '"abc"', "last-modified": "Wed, 27 Aug 2026 10:00:00 GMT" }));
  const res = await proxy(ask("api.weather.gov/alerts/active"));
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("cache-control"), "public, max-age=15");
  assert.equal(res.headers.get("etag"), '"abc"');
  assert.equal(res.headers.get("last-modified"), "Wed, 27 Aug 2026 10:00:00 GMT");
  assert.equal(await res.text(), "hello");
});

test("a matching If-None-Match is answered 304, bodyless, still with the TTL", async () => {
  stubFetch(upstream({ etag: '"abc"' }));
  const res = await proxy(ask("api.weather.gov/alerts/active", { "if-none-match": 'W/"abc"' }));
  assert.equal(res.status, 304);
  assert.equal(res.body, null);
  assert.equal(res.headers.get("cache-control"), "public, max-age=15");
});

test("the client's conditional headers are never forwarded upstream", async () => {
  const calls = stubFetch(upstream({ etag: '"abc"' }));
  await proxy(ask("api.weather.gov/alerts/active", { "if-none-match": '"abc"', cookie: "s=1" }));
  assert.deepEqual(Object.keys(calls[0].init.headers), ["user-agent"]);
});

test("a stale validator still gets the body", async () => {
  stubFetch(upstream({ etag: '"new"' }));
  const res = await proxy(ask("api.weather.gov/alerts/active", { "if-none-match": '"old"' }));
  assert.equal(res.status, 200);
  assert.equal(await res.text(), "hello");
});

test("no upstream validator means no 304 is possible", () => {
  const req = ask("x/y", { "if-none-match": '"abc"' });
  assert.equal(notModified(req, { etag: null, lastModified: null }), false);
});

test("If-Modified-Since compares by date, not by string", () => {
  const at = "Wed, 27 Aug 2026 10:00:00 GMT";
  const later = "Wed, 27 Aug 2026 11:00:00 GMT";
  assert.equal(notModified(ask("x/y", { "if-modified-since": later }), { lastModified: at }), true);
  assert.equal(notModified(ask("x/y", { "if-modified-since": at }), { lastModified: later }), false);
});

test("the TTL table is the one the app's cadences were chosen against", () => {
  assert.equal(cacheSeconds("unidata-nexrad-level2.s3.amazonaws.com", ""), 3600);
  assert.equal(cacheSeconds("unidata-nexrad-level2.s3.amazonaws.com", "?list-type=2"), 15);
  assert.equal(cacheSeconds("maps.dwd.de", "?TIME=2026"), 300);
  assert.equal(cacheSeconds("maps.dwd.de", "?LAYERS=rv"), 15);
  assert.equal(cacheSeconds("tgftp.nws.noaa.gov", ""), 300);
});

test("the trust boundary is unchanged", async () => {
  stubFetch(upstream());
  assert.equal((await proxy(ask("evil.example/x"))).status, 403);
  assert.equal((await proxy(ask("api.weather.gov"))).status, 403);
  const post = new Request("https://example.test/proxy/api.weather.gov/x", { method: "POST" });
  assert.equal((await proxy(post)).status, 403);
});
