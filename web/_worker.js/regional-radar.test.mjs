import { test, afterEach } from "node:test";
import assert from "node:assert/strict";
import { handleRegionalRadar } from "./regional-radar.js";

const originalFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = originalFetch; delete globalThis.caches; });
const key = "private-key-must-never-leave-server";
const stamp = (minutesAgo = 10) => {
  const time = new Date(Date.now() + 9 * 3600000 - minutesAgo * 60000);
  time.setUTCMinutes(Math.floor(time.getUTCMinutes() / 5) * 5, 0, 0);
  return time.toISOString().replace(/\D/g, "").slice(0, 12);
};
const ask = (path, options) => new Request(`https://app.example/regional-radar/${path}`, options);

test("missing credentials report unavailable without contacting KMA", async () => {
  globalThis.fetch = () => { throw new Error("must not fetch"); };
  assert.deepEqual(await (await handleRegionalRadar(ask("status"), {})).json(), { kma: false });
  const response = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), {});
  assert.equal(response.status, 503);
  assert.equal((await response.json()).configured, false);
});

test("only exact recent fixed-product GET requests are accepted", async () => {
  globalThis.fetch = () => { throw new Error("must not fetch"); };
  for (const path of ["kma/grid?time=202602300000", "kma/grid?time=200001010000", `kma/grid?time=${stamp()}&authKey=caller`, `kma/grid?time=${stamp()}&time=${stamp()}`, "kma/catalog?day=../bad"]) {
    assert.equal((await handleRegionalRadar(ask(path), { KMA_API_KEY: key })).status, 400);
  }
  assert.equal((await handleRegionalRadar(ask("status", { method: "POST" }), {})).status, 405);
  assert.equal((await handleRegionalRadar(ask("unknown"), {})).status, 404);
  assert.equal(await handleRegionalRadar(new Request("https://app.example/other"), {}), null);
});

test("credential stays upstream and cached requests use only the public route", async () => {
  const requests = [], cacheKeys = [];
  const time = stamp();
  globalThis.fetch = async (url, init) => {
    requests.push({ url: new URL(url), init });
    return new Response(`# files\nRDR_CMP_HSR_PUB_${time}.bin.gz\n`);
  };
  globalThis.caches = { default: { match: async () => null, put: async (request, response) => { cacheKeys.push(request.url); await response.text(); } } };
  const response = await handleRegionalRadar(ask(`kma/catalog?day=${time.slice(0, 8)}`, { headers: { cookie: "private", authorization: "caller" } }), { KMA_API_KEY: key });
  assert.equal(response.status, 200);
  assert.equal(requests[0].url.origin, "https://apihub.kma.go.kr");
  assert.equal(requests[0].url.searchParams.get("authKey"), key);
  assert.equal(requests[0].url.searchParams.get("cmp"), "HSR");
  assert.equal(requests[0].init.redirect, "manual");
  assert.deepEqual(Object.keys(requests[0].init.headers), ["user-agent"]);
  assert.ok(!cacheKeys[0].includes(key));
  assert.ok(!(await response.text()).includes(key));
});

test("grid streams complete validated values without upstream headers or a cache tee", async () => {
  const data = `-4000,10000,-30000,-25000,-20000,${"0,".repeat(2305 * 2881 - 5)}=\n`;
  globalThis.fetch = async () => new Response(`# server header\n2305,2881,=\n${data}`, { headers: { "x-secret": key } });
  globalThis.caches = { default: {
    match: () => { throw new Error("grid must not read the old edge cache"); },
    put: () => { throw new Error("grid must not tee into the edge cache"); },
  } };
  const response = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), { KMA_API_KEY: key });
  assert.equal(response.status, 200);
  assert.equal(await response.text(), `2305,2881,=\n${data}`);
  assert.equal(response.headers.get("x-secret"), null);
  assert.equal(response.headers.get("cache-control"), "public, max-age=1800");
});

const chunks = values => new ReadableStream({
  pull(controller) {
    if (values.length) controller.enqueue(new TextEncoder().encode(values.shift()));
    else controller.close();
  },
});

async function readUntilFailure(response) {
  const reader = response.body.getReader();
  let text = "", failed = false;
  try {
    while (true) {
      const next = await reader.read();
      if (next.done) break;
      text += new TextDecoder().decode(next.value);
    }
  } catch { failed = true; }
  return { text, failed };
}

test("grid response begins before the upstream body completes", async () => {
  let finished = false;
  globalThis.fetch = async () => new Response(new ReadableStream({
    start(controller) { controller.enqueue(new TextEncoder().encode("2305,2881,=\n0,")); },
    async pull() { await new Promise(() => {}); finished = true; },
  }));
  globalThis.caches = { default: { match: async () => null, put: async (_request, response) => { await response.text(); } } };
  const response = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), { KMA_API_KEY: key });
  assert.equal(response.status, 200);
  assert.equal(finished, false);
  await response.body.cancel();
});

test("grid rejects truncated counts, invalid reflectivity, and credentials across chunk boundaries", async () => {
  for (const value of ["10001", "-4001", "12345678", "1.5", "oops"]) {
    globalThis.fetch = async () => new Response(`2305,2881,=\n${value},`);
    const response = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), { KMA_API_KEY: "12345678" });
    assert.equal(response.status, 502, value);
    assert.ok(!(await response.text()).includes("12345678"));
  }
  globalThis.fetch = async () => new Response("2305,2881,=\n0,0,");
  const incomplete = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), { KMA_API_KEY: key });
  assert.equal((await readUntilFailure(incomplete)).failed, true);
  for (const [secret, pieces] of [
    ["12345678", ["1234", "5678,"]],
    ["1,2,3,4,", ["1,2,", "3,4,"]],
  ]) {
    globalThis.fetch = async () => new Response(chunks(["2305,2881,=\n0,", ...pieces]));
    const response = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), { KMA_API_KEY: secret });
    const result = await readUntilFailure(response);
    assert.equal(result.failed, true);
    assert.ok(!result.text.includes(secret));
  }
});

test("redirects, oversized bodies, and provider errors never expose credentials", async () => {
  for (const response of [
    new Response(null, { status: 302, headers: { location: `https://evil.test/${key}` } }),
    new Response(`<html>Invalid key: ${key}</html>`),
    new Response("body", { headers: { "content-length": String(49 * 1024 * 1024) } }),
  ]) {
    globalThis.fetch = async () => response;
    const result = await handleRegionalRadar(ask(`kma/grid?time=${stamp()}`), { KMA_API_KEY: key });
    assert.equal(result.status, 502);
    assert.ok(!(await result.text()).includes(key));
  }
  globalThis.fetch = async () => { throw new Error(`request failed: ${key}`); };
  const result = await handleRegionalRadar(ask(`kma/catalog?day=${stamp().slice(0, 8)}`), { KMA_API_KEY: key });
  assert.equal(result.status, 502);
  assert.ok(!(await result.text()).includes(key));
  globalThis.fetch = async () => new Response(`# failed URL?authKey=${encodeURIComponent(key)}`);
  const encodedError = await handleRegionalRadar(ask(`kma/catalog?day=${stamp().slice(0, 8)}`), { KMA_API_KEY: key });
  assert.equal(encodedError.status, 502);
});
