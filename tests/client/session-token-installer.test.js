import { expect, it, vi } from 'vitest';
import { installSessionToken } from '../../packages/session/src/session-token.js';
function storage(seed = {}) { const values = new Map(Object.entries(seed)); return { getItem: key => values.get(key) || null, setItem: (key, value) => values.set(key, value), values }; }
let entropy = 0;
function windowWith(local = storage(), session = storage(), reload = false) {
  const win = new EventTarget();
  return Object.assign(win, { localStorage: local, sessionStorage: session,
    crypto: { getRandomValues: array => { array.fill(++entropy); return array; } },
    setInterval: vi.fn(() => 42), clearInterval: vi.fn(),
    performance: { getEntriesByType: () => [{ type: reload ? 'reload' : 'navigate' }] } });
}
it('installs first-tab adoption, isolates concurrent tabs, and keeps reload identity', () => {
  const shared = 'a'.repeat(32), local = storage({ 'session-token': shared });
  const first = windowWith(local);
  expect(installSessionToken(first)).toBe(shared);
  const second = windowWith(local);
  expect(installSessionToken(second)).not.toBe(shared);
  expect(local.getItem('session-token')).toBe(shared);
  const reload = windowWith(local, first.sessionStorage, true);
  expect(installSessionToken(reload)).toBe(shared);
});
it.each(['getItem', 'setItem'])('survives storage %s throwing and returns a shaped identity', method => {
  const denied = { getItem: () => null, setItem: () => {} }; denied[method] = () => { throw new Error('denied'); };
  expect(installSessionToken(windowWith(denied, denied))).toMatch(/^[0-9a-f]{32}$/);
});
it('handles storage getters throwing', () => {
  const win = windowWith(); Object.defineProperty(win, 'localStorage', { get() { throw new Error('denied'); } });
  expect(installSessionToken(win)).toMatch(/^[0-9a-f]{32}$/);
});
it('prunes corrupt registry, honors application keys, and stops and renews the lease on page transitions', () => {
  const local = storage({ tabs: 'broken' }), win = windowWith(local);
  const token = installSessionToken(win, { tabKey: 'tab', sharedKey: 'identity', registryKey: 'tabs' });
  expect(win.sessionStorage.getItem('tab')).toBe(token); expect(local.getItem('identity')).toBe(token);
  expect(local.getItem('session-token')).toBeNull();
  expect(Object.values(JSON.parse(local.getItem('tabs')))).toHaveLength(1);
  win.dispatchEvent(new Event('pagehide'));
  expect(JSON.parse(local.getItem('tabs'))).toEqual({}); expect(win.clearInterval).toHaveBeenCalledWith(42);
  win.dispatchEvent(new Event('pageshow'));
  expect(Object.values(JSON.parse(local.getItem('tabs')))).toHaveLength(1); expect(win.setInterval).toHaveBeenCalledTimes(2);
});
