import test from "node:test";
import assert from "node:assert/strict";
import worker from "./index.js";

test("missing bundle assets cannot become cached HTML, while valid JS passes through", async () => {
  for (const [status, type, expected] of [[200, "text/html", 404], [404, "text/plain", 404], [200, "application/javascript", 200]]) {
    const result = await worker.fetch(new Request("https://app.hookecho.io/dist/app.js"), {
      ASSETS: { fetch: async () => new Response("body", { status, headers: { "content-type": type } }) },
    });
    assert.equal(result.status, expected);
    if (expected === 404) assert.equal(result.headers.get("cache-control"), "no-store");
  }
});

test("asset revalidation retains 304 responses", async () => {
  const result = await worker.fetch(new Request("https://app.hookecho.io/decode-bridge.js"), {
    ASSETS: { fetch: async () => new Response(null, { status: 304, headers: { etag: '"asset"' } }) },
  });
  assert.equal(result.status, 304);
  assert.equal(result.headers.get("etag"), '"asset"');
});
