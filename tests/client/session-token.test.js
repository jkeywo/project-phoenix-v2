import { describe, it, expect } from 'vitest';
import {
  mintToken,
  pruneRegistry,
  isClaimedByOtherTab,
  decideToken,
  REGISTRY_TTL_MS,
} from '../../gui/session-token.js';

const seq = (...bytes) => (arr) => { arr.set(bytes.slice(0, arr.length)); return arr; };

describe('mintToken', () => {
  it('produces 32 lowercase hex chars', () => {
    const t = mintToken(seq(0x0a, 0xff, 0x00, 0x10));
    expect(t).toMatch(/^[0-9a-f]{32}$/);
    expect(t.startsWith('0aff0010')).toBe(true);
  });
});

describe('pruneRegistry', () => {
  it('keeps fresh entries and drops stale ones', () => {
    const now = 10_000;
    const reg = {
      a: { token: 'x', ts: now - 1000 },                 // fresh
      b: { token: 'y', ts: now - REGISTRY_TTL_MS - 1 },  // stale
    };
    expect(pruneRegistry(reg, now)).toEqual({ a: { token: 'x', ts: now - 1000 } });
  });

  it('does not mutate its input and tolerates junk', () => {
    const reg = { a: { token: 'x', ts: 0 }, b: null, c: { token: 'z' } };
    const out = pruneRegistry(reg, 0);
    expect(out).toEqual({ a: { token: 'x', ts: 0 } });
    expect(reg.b).toBe(null); // untouched
  });
});

describe('isClaimedByOtherTab', () => {
  const now = 5000;
  const reg = { tabA: { token: 'shared', ts: now - 500 } };

  it('true when another live tab holds the token', () => {
    expect(isClaimedByOtherTab(reg, 'shared', 'tabB', now)).toBe(true);
  });

  it('false when only our own tab holds it', () => {
    expect(isClaimedByOtherTab(reg, 'shared', 'tabA', now)).toBe(false);
  });

  it('false when the holder lease has expired', () => {
    const stale = { tabA: { token: 'shared', ts: now - REGISTRY_TTL_MS - 1 } };
    expect(isClaimedByOtherTab(stale, 'shared', 'tabB', now)).toBe(false);
  });

  it('false for a null/absent token', () => {
    expect(isClaimedByOtherTab(reg, null, 'tabB', now)).toBe(false);
    expect(isClaimedByOtherTab({}, 'shared', 'tabB', now)).toBe(false);
  });
});

describe('decideToken', () => {
  it('reuses this tab\'s own token on reload, persisting nothing', () => {
    expect(decideToken({ tabToken: 'mine', sharedToken: 'shared', sharedClaimed: false, freshToken: 'fresh' }))
      .toEqual({ token: 'mine', storeAsTab: false, storeAsShared: false });
  });

  it('mints fresh when a duplicated tab inherits a token already live elsewhere', () => {
    expect(decideToken({
      tabToken: 'mine',
      tabTokenClaimed: true,
      isReload: false,
      sharedToken: 'mine',
      sharedClaimed: true,
      freshToken: 'fresh',
    })).toEqual({ token: 'fresh', storeAsTab: true, storeAsShared: false });
  });

  it('keeps this tab token on reload even while its old lease is expiring', () => {
    expect(decideToken({
      tabToken: 'mine',
      tabTokenClaimed: true,
      isReload: true,
      sharedToken: 'mine',
      sharedClaimed: true,
      freshToken: 'fresh',
    })).toEqual({ token: 'mine', storeAsTab: false, storeAsShared: false });
  });

  it('first/only tab adopts the persistent shared token', () => {
    expect(decideToken({ tabToken: null, sharedToken: 'shared', sharedClaimed: false, freshToken: 'fresh' }))
      .toEqual({ token: 'shared', storeAsTab: true, storeAsShared: false });
  });

  it('a concurrent tab mints fresh without overwriting the shared token', () => {
    expect(decideToken({ tabToken: null, sharedToken: 'shared', sharedClaimed: true, freshToken: 'fresh' }))
      .toEqual({ token: 'fresh', storeAsTab: true, storeAsShared: false });
  });

  it('a brand-new origin mints fresh and seeds the persistent token', () => {
    expect(decideToken({ tabToken: null, sharedToken: null, sharedClaimed: false, freshToken: 'fresh' }))
      .toEqual({ token: 'fresh', storeAsTab: true, storeAsShared: true });
  });
});
