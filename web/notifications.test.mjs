import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const rust = await readFile(new URL('../crates/hookecho/src/notify.rs', import.meta.url), 'utf8');
const source = rust.match(/inline_js = r#"\n(export async function showBrowserNotification[\s\S]*?)"#/)[1].replace('export ', '');
function browser({permission = 'granted', mobile = false, workers, timers = {setTimeout, clearTimeout}} = {}) {
  const calls = [];
  function Notification(title, options) {
    if (mobile) throw new TypeError('Constructor unsupported');
    calls.push({title, options});
  }
  Notification.permission = permission;
  const context = vm.createContext({Notification, navigator: {serviceWorker: workers}, console: {warn() {}}, ...timers});
  vm.runInContext(source, context);
  return {show: context.showBrowserNotification, calls};
}

test('mobile alerts use the service worker and preserve summary tags', async () => {
  const calls = [];
  const registration = {active: {}, showNotification: async (title, options) => calls.push({title, options})};
  const b = browser({mobile: true, workers: {getRegistration: async () => registration}});
  assert.equal(await b.show('Warning', 'Details', 'summary'), true);
  assert.equal(b.calls.length, 0);
  assert.equal(calls[0].options.tag, 'summary');
  assert.equal(calls[0].options.body, 'Details');
  assert.equal(calls[0].options.icon, '/icon-192.png');
});

test('first-install notifications wait for worker activation', async () => {
  let activate, delivered = false;
  const ready = new Promise(resolve => activate = resolve);
  const b = browser({mobile: true, workers: {getRegistration: async () => ({active: null}), ready}});
  const pending = b.show('Warning', 'Details');
  await Promise.resolve();
  assert.equal(delivered, false);
  activate({showNotification: async () => { delivered = true; }});
  assert.equal(await pending, true);
  assert.equal(delivered, true);
});

test('denied permissions never prompt or attempt delivery', async () => {
  const b = browser({permission: 'denied', workers: {getRegistration: () => assert.fail('must not deliver')}});
  assert.equal(await b.show('Warning', 'Details'), false);
  assert.equal(b.calls.length, 0);
});

test('desktop notification fallback works without a worker or after worker failure', async () => {
  for (const workers of [undefined, {getRegistration: async () => undefined},
    {getRegistration: async () => { throw new Error('unavailable'); }}]) {
    const b = browser({workers});
    assert.equal(await b.show('Warning', 'Details'), true);
    assert.equal(b.calls.length, 1);
    assert.equal(b.calls[0].options.tag, undefined);
  }
});

test('delivery failure is surfaced to the Rust logger', async () => {
  const b = browser({mobile: true});
  await assert.rejects(b.show('Warning', 'Details'), /unsupported/);
});

test('failed worker activation cannot hold desktop notifications forever', async () => {
  let cleared = false;
  const b = browser({workers: {getRegistration: async () => ({active: null}), ready: new Promise(() => {})},
    timers: {setTimeout: callback => { queueMicrotask(callback); return 1; }, clearTimeout: id => { cleared = id === 1; }}});
  assert.equal(await b.show('Warning', 'Details'), true);
  assert.equal(cleared, true);
  assert.equal(b.calls.length, 1);
});
