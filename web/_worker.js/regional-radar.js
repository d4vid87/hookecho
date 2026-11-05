// The only KMA credential lives in the server environment. Clients request fixed products;
// neither upstream URLs nor caller-supplied credentials are accepted.
const PREFIX = "/regional-radar/";
const MAX_GRID_BYTES = 48 * 1024 * 1024;
const MAX_CATALOG_BYTES = 1024 * 1024;

const reply = (body, status = 200) => new Response(JSON.stringify(body), {
  status,
  headers: { "content-type": "application/json", "cache-control": "no-store" },
});

function koreanTime(value, dayOnly) {
  if (!(dayOnly ? /^\d{8}$/ : /^\d{12}$/).test(value)) return null;
  const year = Number(value.slice(0, 4)), month = Number(value.slice(4, 6));
  const day = Number(value.slice(6, 8));
  const hour = dayOnly ? 0 : Number(value.slice(8, 10));
  const minute = dayOnly ? 0 : Number(value.slice(10, 12));
  const date = new Date(Date.UTC(year, month - 1, day, hour, minute));
  if (date.getUTCFullYear() !== year || date.getUTCMonth() !== month - 1
      || date.getUTCDate() !== day || date.getUTCHours() !== hour
      || date.getUTCMinutes() !== minute || minute % 5 !== 0) return null;
  return date.getTime() - 9 * 60 * 60 * 1000;
}

async function catalogBody(response, key) {
  const reader = response.body?.getReader();
  if (!reader) throw new Error("empty catalog");
  const chunks = [];
  let size = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > MAX_CATALOG_BYTES) { await reader.cancel(); throw new Error("catalog too large"); }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  if (text.includes(key) || /authkey|servicekey/i.test(text) || /<\s*(?:html|!doctype|error|response)/i.test(text)
      || /(?:invalid|denied|unauthorized|인증키|오류)/i.test(text)) throw new Error("invalid catalog");
  return text;
}

// Validate the fixed grid header, each physical value and the exact cell count. A short
// lookbehind withholds possible credential prefixes until the next upstream chunk arrives.
async function gridBody(response, key) {
  const reader = response.body?.getReader();
  if (!reader) throw new Error("empty grid");
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let prefix = "", size = 0, remainder;
  while (remainder === undefined) {
    const { value, done } = await reader.read();
    if (done) throw new Error("missing grid header");
    size += value.length;
    if (size > MAX_GRID_BYTES) { await reader.cancel(); throw new Error("grid too large"); }
    prefix += decoder.decode(value, { stream: true });
    while (prefix.includes("\n")) {
      const end = prefix.indexOf("\n");
      const line = prefix.slice(0, end).trim();
      prefix = prefix.slice(end + 1);
      if (!line || line.startsWith("#")) continue;
      if (!/^2305\s*,\s*2881\s*,\s*=$/.test(line)) {
        await reader.cancel(); throw new Error("unexpected grid header");
      }
      remainder = prefix;
      break;
    }
    if (remainder === undefined && size > 64 * 1024) { await reader.cancel(); throw new Error("grid header too large"); }
  }
  const encoder = new TextEncoder();
  const count = 2305 * 2881;
  let digits = 0, signed = false, negative = false, magnitude = 0, cells = 0, held = "";
  const finishToken = () => {
    if (!digits && !signed) return;
    const value = negative ? -magnitude : magnitude;
    if (!digits || (!(value >= -4000 && value <= 10000) && value !== -30000 && value !== -25000 && value !== -20000)) {
      throw new Error("invalid reflectivity");
    }
    if (++cells > count) throw new Error("too many cells");
    digits = 0; signed = false; negative = false; magnitude = 0;
  };
  const validate = text => {
    // Numeric state avoids allocating six million token strings or running a regex per byte.
    for (let i = 0; i < text.length; i++) {
      const code = text.charCodeAt(i);
      if (code >= 48 && code <= 57) {
        if (++digits + Number(signed) > 6) throw new Error("invalid reflectivity");
        magnitude = magnitude * 10 + code - 48;
      } else if (code === 43 || code === 45) {
        if (digits || signed) throw new Error("invalid reflectivity");
        signed = true; negative = code === 45;
      } else if (code === 44 || code === 61 || code === 32 || code === 9 || code === 10 || code === 13) {
        finishToken();
      } else {
        throw new Error("invalid grid character");
      }
    }
  };
  const emit = (controller, text, final = false) => {
    const pending = held + text;
    if (pending.includes(key)) throw new Error("invalid grid");
    const end = final ? pending.length : Math.max(0, pending.length - key.length + 1);
    if (end) controller.enqueue(encoder.encode(pending.slice(0, end)));
    held = pending.slice(end);
  };
  try {
    if (size > MAX_GRID_BYTES) throw new Error("grid too large");
    validate(remainder);
    if (("2305,2881,=\n" + remainder).includes(key)) throw new Error("invalid grid");
  } catch {
    await reader.cancel(); throw new Error("invalid grid");
  }
  return new ReadableStream({
    start(controller) { emit(controller, "2305,2881,=\n" + remainder); },
    async pull(controller) {
      try {
        const { value, done } = await reader.read();
        if (done) {
          const tail = decoder.decode();
          validate(tail);
          finishToken();
          if (cells !== count) throw new Error("incomplete grid");
          emit(controller, tail, true);
          controller.close();
          return;
        }
        size += value.length;
        const text = decoder.decode(value, { stream: true });
        if (size > MAX_GRID_BYTES) throw new Error("grid too large");
        validate(text);
        emit(controller, text);
      } catch {
        await reader.cancel().catch(() => {});
        controller.error(new Error("KMA radar stream failed"));
      }
    },
    cancel(reason) { return reader.cancel(reason); },
  });
}

/** Return null for unrelated routes. No credentials or upstream error details leave this module. */
export async function handleRegionalRadar(request, env) {
  const url = new URL(request.url);
  if (!url.pathname.startsWith(PREFIX)) return null;
  if (request.method !== "GET") return reply({ error: "GET required" }, 405);
  const key = typeof env.KMA_API_KEY === "string" ? env.KMA_API_KEY.trim() : "";
  const configured = key.length >= 8 && key.length <= 512;
  if (url.pathname === `${PREFIX}status` && !url.search) return reply({ kma: configured });
  const catalog = url.pathname === `${PREFIX}kma/catalog`;
  const grid = url.pathname === `${PREFIX}kma/grid`;
  if (!catalog && !grid) return reply({ error: "Unknown radar route" }, 404);
  const parameter = catalog ? "day" : "time";
  const entries = [...url.searchParams.entries()];
  if (entries.length !== 1 || entries[0][0] !== parameter) return reply({ error: "Invalid radar parameters" }, 400);
  const value = entries[0][1];
  const timestamp = koreanTime(value, catalog);
  const now = Date.now();
  if (timestamp === null || timestamp > now + 60_000 || timestamp < now - (catalog ? 72 : 48) * 3_600_000) {
    return reply({ error: "Radar time is outside the recent observation window" }, 400);
  }
  if (!configured) return reply({ error: "KMA radar is not configured on this server", configured: false }, 503);
  // Awaiting Cache.put on one tee branch would buffer the entire grid in its unread sibling.
  // Cache only the small catalog here; grid clients still receive normal browser cache headers.
  const cache = catalog ? globalThis.caches?.default : undefined;
  // A canonical, credential-free cache key prevents secrets entering shared cache metadata.
  const cacheKey = new Request(`${url.origin}${url.pathname}?${parameter}=${value}`);
  try {
    const cached = await cache?.match(cacheKey);
    if (cached) return cached;
    const upstream = new URL(catalog
      ? "https://apihub.kma.go.kr/api/typ01/url/rdr_cmp_file_list.php"
      : "https://apihub.kma.go.kr/api/typ01/cgi-bin/url/nph-rdr_cmp1_api");
    upstream.searchParams.set("cmp", "HSR");
    upstream.searchParams.set("tm", value);
    if (grid) {
      for (const [name, parameterValue] of Object.entries({ qcd: "MSK", obs: "ECHO", map: "HB", disp: "A" })) {
        upstream.searchParams.set(name, parameterValue);
      }
    }
    upstream.searchParams.set("authKey", key);
    const response = await fetch(upstream.toString(), {
      redirect: "manual", signal: AbortSignal.timeout(30_000),
      headers: { "user-agent": "HookEcho worldwide radar" },
      cf: { cacheEverything: false, cacheTtl: 0 },
    });
    if (!response.ok || Number(response.headers.get("content-length") || 0) > (catalog ? MAX_CATALOG_BYTES : MAX_GRID_BYTES)) {
      await response.body?.cancel();
      return reply({ error: "KMA radar is temporarily unavailable" }, 502);
    }
    const body = catalog ? await catalogBody(response, key) : await gridBody(response, key);
    const result = new Response(body, { headers: {
      "content-type": "text/plain; charset=utf-8",
      "cache-control": `public, max-age=${catalog ? 120 : 1800}`,
      "x-content-type-options": "nosniff",
    } });
    if (cache) await cache.put(cacheKey, result.clone());
    return result;
  } catch {
    return reply({ error: "KMA radar is temporarily unavailable" }, 502);
  }
}
