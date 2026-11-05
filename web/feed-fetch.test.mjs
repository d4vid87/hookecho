import {test} from 'node:test';
import assert from 'node:assert/strict';
import {feedFetch, retrySeconds} from './feed-fetch.js';

const origin = 'https://app.example';
test('Retry-After accepts seconds and HTTP dates with bounded, safe fallbacks', () => {
  const now = Date.parse('Wed, 04 Nov 2026 12:00:00 GMT');
  assert.equal(retrySeconds('12', now), 12);
  assert.equal(retrySeconds('Wed, 04 Nov 2026 12:00:45 GMT', now), 45);
  assert.equal(retrySeconds('Wed, 04 Nov 2026 11:59:00 GMT', now), 0);
  assert.equal(retrySeconds('999999999', now), 86400);
  for (const value of [null, '', '-3', '1.5', 'garbage']) assert.equal(retrySeconds(value, now), 30);
});
test('provider cooldown spans tiles, expires, and leaves other hosts and methods alone', async () => {
  let now = 0, calls = 0;
  const fetch = feedFetch(async () => {
    calls++;
    return new Response('limited', {status: 429, headers: {'retry-after': '10'}});
  }, origin, () => now);
  await fetch('/proxy/maps.dwd.de/tile1');
  now = 2500;
  const blocked = await fetch('/proxy/maps.dwd.de/tile2');
  assert.equal(calls, 1);
  assert.equal(blocked.status, 429);
  assert.equal(blocked.url, `${origin}/proxy/maps.dwd.de/tile2`);
  assert.equal(blocked.headers.get('retry-after'), '8');
  await fetch('/proxy/other.example/tile');
  await fetch('/dist/app.js');
  await fetch('https://elsewhere.example/proxy/maps.dwd.de/tile');
  await fetch('/proxy/maps.dwd.de/tile', {method: 'POST'});
  assert.equal(calls, 5);
  now = 10000;
  await fetch('/proxy/maps.dwd.de/tile2');
  assert.equal(calls, 6);
});
test('Request and init signals remain cancellable during a cooldown', async () => {
  let calls = 0;
  const fetch = feedFetch(async () => {calls++; return new Response('', {status: 503});}, origin);
  const request = new Request(`${origin}/proxy/api.weather.gov/alerts`);
  await fetch(request);
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(fetch(request, {signal: controller.signal}), {name: 'AbortError'});
  await assert.rejects(fetch(new Request(request, {signal: controller.signal})), {name: 'AbortError'});
  assert.equal(calls, 1);
});
test('a concurrent shorter response cannot shorten the existing cooldown', async () => {
  const resolvers = [];
  let now = 0;
  const fetch = feedFetch(() => new Promise(resolve => resolvers.push(resolve)), origin, () => now);
  const first = fetch('/proxy/maps.dwd.de/a'), second = fetch('/proxy/maps.dwd.de/b');
  resolvers[1](new Response('', {status: 503, headers: {'retry-after': '60'}})); await second;
  resolvers[0](new Response('', {status: 429, headers: {'retry-after': '10'}})); await first;
  now = 20000;
  const response = await fetch('/proxy/maps.dwd.de/c');
  assert.equal(response.status, 503);
  assert.equal(response.headers.get('retry-after'), '40');
  assert.equal(resolvers.length, 2);
});
