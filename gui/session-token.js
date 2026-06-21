// Per-tab session-token resolution.
//
// The session token is the player's identity on the host (see server.html
// `tokenConns` routing and the DUPLICATE TOKEN warning). Historically the
// client stored it in `localStorage`, which is shared across every tab and
// window of the same origin — so opening two console tabs on one desktop made
// them read the SAME token. The host then routes all state to whichever tab
// identified last, and the earlier tab becomes a "ghost" (its taps succeed but
// show no feedback). That is why multiple clients on one machine can't all be
// connected at once.
//
// Fix: give each TAB its own token via `sessionStorage` (per-tab, survives a
// reload), while preserving the documented cross-restart reconnect for the
// common single-phone case:
//
//   - A reloaded tab reuses its own sessionStorage token.
//   - The first/only tab ADOPTS the persistent localStorage token, so a single
//     phone still reconnects onto its station after a full browser restart.
//   - Additional CONCURRENT tabs mint a fresh token instead of clobbering the
//     shared one, so each connects as a distinct player.
//
// Liveness ("is another tab already using the shared token right now?") is
// tracked with a short-TTL heartbeat registry in localStorage, so the question
// is answerable synchronously at load and stale entries self-expire (mobile
// `pagehide`/`unload` is unreliable, so we never rely on explicit cleanup).

export const TAB_KEY = 'session-token';        // sessionStorage: this tab's token
export const SHARED_KEY = 'session-token';     // localStorage: persistent token
export const REGISTRY_KEY = 'phoenix-live-tabs';
export const HEARTBEAT_MS = 2000;
export const REGISTRY_TTL_MS = 6000;

// Pure: 32 hex chars from a getRandomValues-compatible source, matching the
// inline minting in client.html so token shape is unchanged.
export function mintToken(getRandomValues) {
  const bytes = getRandomValues(new Uint8Array(16));
  return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

// Pure: drop registry entries older than `ttl`. `registry` is a plain object
// keyed by tabId → { token, ts }. Returns a new object (never mutates input).
export function pruneRegistry(registry, now, ttl = REGISTRY_TTL_MS) {
  const out = {};
  for (const [tabId, entry] of Object.entries(registry || {})) {
    if (entry && typeof entry.ts === 'number' && now - entry.ts <= ttl) {
      out[tabId] = entry;
    }
  }
  return out;
}

// Pure: is `token` currently held by a live tab OTHER than `myTabId`?
export function isClaimedByOtherTab(registry, token, myTabId, now, ttl = REGISTRY_TTL_MS) {
  if (!token) return false;
  for (const [tabId, entry] of Object.entries(registry || {})) {
    if (tabId === myTabId) continue;
    if (!entry || entry.token !== token) continue;
    if (now - entry.ts <= ttl) return true;
  }
  return false;
}

// Pure: decide which token this tab should use and what to persist.
//
//   tabToken      — sessionStorage token for this tab (null on first load)
//   sharedToken   — localStorage persistent token (null if none yet)
//   sharedClaimed — is `sharedToken` currently held by another live tab?
//   freshToken    — a freshly minted token to use when we can't adopt
//
// Returns { token, storeAsTab, storeAsShared }:
//   storeAsTab    — write `token` to sessionStorage (skip when reusing it)
//   storeAsShared — write `token` to localStorage (only when seeding the first
//                   persistent token, so we never overwrite another tab's)
export function decideToken({ tabToken, sharedToken, sharedClaimed, freshToken }) {
  // Reload of this same tab: reuse our own token untouched.
  if (tabToken) {
    return { token: tabToken, storeAsTab: false, storeAsShared: false };
  }
  // First/only tab: adopt the persistent shared token if no live tab holds it.
  if (sharedToken && !sharedClaimed) {
    return { token: sharedToken, storeAsTab: true, storeAsShared: false };
  }
  // A concurrent tab (shared token is in use) or a brand-new origin: mint our
  // own. Seed localStorage only when there is no persistent token yet, so a
  // future single-tab restart still reconnects.
  return { token: freshToken, storeAsTab: true, storeAsShared: !sharedToken };
}

// Impure: resolve this tab's token against real Storage objects and start the
// heartbeat that keeps the live-tab registry fresh. Returns the token string.
//
// `win` defaults to the global window; injectable for tests. Falls back to a
// plain in-memory token if storage is unavailable (private mode, etc.).
export function installSessionToken(win = (typeof window !== 'undefined' ? window : undefined)) {
  const getRandomValues = (arr) => win.crypto.getRandomValues(arr);
  const freshToken = mintToken(getRandomValues);

  let sessionStore, localStore;
  try { sessionStore = win.sessionStorage; localStore = win.localStorage; } catch (_) {}
  if (!sessionStore || !localStore) return freshToken;

  const now = Date.now();
  const tabId = mintToken(getRandomValues);

  const readRegistry = () => {
    try { return pruneRegistry(JSON.parse(localStore.getItem(REGISTRY_KEY)) || {}, Date.now()); }
    catch (_) { return {}; }
  };

  const tabToken = sessionStore.getItem(TAB_KEY);
  const sharedToken = localStore.getItem(SHARED_KEY);
  const sharedClaimed = isClaimedByOtherTab(readRegistry(), sharedToken, tabId, now);

  const { token, storeAsTab, storeAsShared } =
    decideToken({ tabToken, sharedToken, sharedClaimed, freshToken });

  try {
    if (storeAsTab) sessionStore.setItem(TAB_KEY, token);
    if (storeAsShared) localStore.setItem(SHARED_KEY, token);
  } catch (_) {}

  // Heartbeat: keep this tab's lease fresh so other tabs can see the token is
  // taken. Pruning on each write keeps the registry from growing unbounded.
  const beat = () => {
    try {
      const reg = readRegistry();
      reg[tabId] = { token, ts: Date.now() };
      localStore.setItem(REGISTRY_KEY, JSON.stringify(reg));
    } catch (_) {}
  };
  beat();
  if (typeof win.setInterval === 'function') win.setInterval(beat, HEARTBEAT_MS);

  const drop = () => {
    try {
      const reg = readRegistry();
      delete reg[tabId];
      localStore.setItem(REGISTRY_KEY, JSON.stringify(reg));
    } catch (_) {}
  };
  if (typeof win.addEventListener === 'function') win.addEventListener('pagehide', drop);

  return token;
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.installSessionToken = installSessionToken;
}
