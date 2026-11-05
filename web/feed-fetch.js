// A provider's Retry-After applies to its other feed requests too, not just one tile.
export function retrySeconds(value, now = Date.now()) {
  const text = (value ?? '').trim();
  const seconds = /^\d+$/.test(text) ? Number(text)
    : /^[A-Za-z]{3},/.test(text) ? (Date.parse(text) - now) / 1000 : NaN;
  return Number.isFinite(seconds) ? Math.min(86400, Math.max(0, Math.ceil(seconds))) : 30;
}

export function feedFetch(fetchImpl, origin, now = Date.now) {
  const cooldowns = new Map();
  return async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : input, origin);
    const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
    const host = url.origin === origin && method.toUpperCase() === 'GET'
      ? /^\/proxy\/([^/]+)\//.exec(url.pathname)?.[1] : null;
    if (!host) return fetchImpl(input, init);
    const signal = init?.signal ?? (input instanceof Request ? input.signal : null);
    if (signal?.aborted) throw signal.reason ?? new DOMException('Aborted', 'AbortError');
    const time = now();
    for (const [key, item] of cooldowns) if (item.until <= time) cooldowns.delete(key);
    const pending = cooldowns.get(host);
    if (pending) {
      const response = new Response('{"error":"provider temporarily unavailable"}', {
      status: pending.status, headers: {
        'content-type': 'application/json', 'cache-control': 'no-store',
        'retry-after': String(Math.ceil((pending.until - time) / 1000)),
      },
      });
      // reqwest's WASM client requires the URL normally attached by browser fetch.
      Object.defineProperty(response, 'url', {value: url.href});
      return response;
    }
    const response = await fetchImpl(input, init);
    if (response.status === 429 || response.status === 503) {
      const until = now() + retrySeconds(response.headers.get('retry-after'), now()) * 1000;
      // An earlier concurrent request must not shorten a newer provider cooldown.
      if (until > (cooldowns.get(host)?.until ?? 0)) {
        if (cooldowns.size >= 128 && !cooldowns.has(host)) cooldowns.delete(cooldowns.keys().next().value);
        cooldowns.set(host, {until, status: response.status});
      }
    }
    return response;
  };
}
