/**
 * Exclusive browser-host identity for peer-local save catalogues (#865).
 *
 * localStorage is origin-wide. Phoenix can run several simulation hosts under
 * one origin/profile, so a storage namespace may be reused only while this
 * document owns an exclusive Web Lock for it. Web Locks are the lifecycle
 * primitive here: they are atomic, do not expire while a tab is throttled or
 * asleep, and cannot be cloned with sessionStorage by a duplicated tab.
 *
 * The chosen identity is remembered in sessionStorage and, for the common
 * single-host case, in localStorage for a later browser restart. Reuse normally
 * requires the lock. When the LockManager is unavailable or denies requests,
 * a same-origin SharedWorker serializes live claims; a one-shot sessionStorage
 * ticket transfers one claim to this peer's successor document. If neither
 * primitive is available, isolation wins: the page uses a fresh in-memory
 * identity and durable Start fails closed.
 */

import { mintToken } from './session-token.js';

// `BROWSER_SAVE_TAB_KEY` is also read at the Rust/WASM bridge boundary. Keep
// the literal in sync with `BROWSER_SAVE_IDENTITY_KEY` in server/bridge.rs.
export const BROWSER_SAVE_TAB_KEY = 'phoenix-save-peer-id';
export const BROWSER_SAVE_SHARED_KEY = 'phoenix-save-peer-id';
export const BROWSER_SAVE_HANDOFF_KEY = 'phoenix-save-peer-handoff';
export const BROWSER_SAVE_DIRECTORY_PREFIX = 'phoenix-save-peer-id:';
export const BROWSER_SAVE_IDENTITY_PROPERTY = '__phoenixSavePeerIdentity';
export const BROWSER_SAVE_LOCK_PREFIX = 'phoenix-save-peer:';
export const HANDOFF_GRACE_MS = 400;

const COORDINATOR_NAME = 'phoenix-save-peer-identities';
const COORDINATOR_URL = new URL('./browser-save-identity-worker.js', import.meta.url);

const TOKEN_PATTERN = /^[0-9a-f]{32}$/;
const installations = new WeakMap();

// A granted `navigator.locks.request()` promise stays pending for as long as
// its callback holds the lock. Root it explicitly for the whole document
// lifetime, and consume either settlement path so aborted queued requests never
// surface as unhandled rejections.
const heldRequests = new Set();

export function validBrowserSaveIdentity(value) {
  return typeof value === 'string' && TOKEN_PATTERN.test(value);
}

function safeStorage(win, property) {
  try { return win[property] || null; } catch (_) { return null; }
}

function safeGet(storage, key) {
  if (!storage) return { readable: false, value: null };
  try { return { readable: true, value: storage.getItem(key) }; }
  catch (_) { return { readable: false, value: null }; }
}

function safeSet(storage, key, value) {
  if (!storage) return false;
  try { storage.setItem(key, value); return true; }
  catch (_) { return false; }
}

function safeRemove(storage, key) {
  if (!storage) return false;
  try { storage.removeItem(key); return true; }
  catch (_) { return false; }
}

function rootLockRequest(request) {
  if (!request || typeof request.then !== 'function') return;
  heldRequests.add(request);
  request.then(
    () => heldRequests.delete(request),
    () => heldRequests.delete(request),
  );
}

function lockManager(win) {
  try {
    const manager = win.navigator && win.navigator.locks;
    return manager && typeof manager.request === 'function' ? manager : null;
  } catch (_) {
    return null;
  }
}

/**
 * Ask for one lock and resolve when its callback learns whether it was granted.
 * A granted callback deliberately returns an unresolved promise until
 * `handle.release()`; the LockManager request promise is rooted above.
 */
function requestLock(manager, name, options = {}) {
  return new Promise((resolve) => {
    let callbackRan = false;
    let request;
    try {
      request = manager.request(name, options, (lock) => {
        callbackRan = true;
        if (!lock) {
          resolve(null);
          return undefined;
        }

        let releaseLock;
        let released = false;
        const held = new Promise((release) => { releaseLock = release; });
        resolve({
          kind: 'lock',
          name,
          release() {
            if (released) return;
            released = true;
            releaseLock();
          },
        });
        return held;
      });
    } catch (_) {
      resolve(null);
      return;
    }

    rootLockRequest(request);
    Promise.resolve(request).then(
      () => { if (!callbackRan) resolve(null); },
      () => resolve(null),
    );
  });
}

async function claimIdentity(win, manager, identity, handoffMs) {
  const name = BROWSER_SAVE_LOCK_PREFIX + identity;
  const immediate = await requestLock(manager, name, { mode: 'exclusive', ifAvailable: true });
  if (immediate) return immediate;

  // A same-tab navigation can instantiate the new document just before the old
  // one releases its lock. Queue briefly for that handoff. A duplicated/live
  // peer keeps the lock, the AbortSignal ends the wait, and the caller mints a
  // different identity. Abort rejection is consumed in `requestLock`.
  const AbortControllerClass = win.AbortController || globalThis.AbortController;
  if (!AbortControllerClass || handoffMs <= 0) return null;
  const controller = new AbortControllerClass();
  const setTimer = typeof win.setTimeout === 'function'
    ? win.setTimeout.bind(win)
    : globalThis.setTimeout;
  const clearTimer = typeof win.clearTimeout === 'function'
    ? win.clearTimeout.bind(win)
    : globalThis.clearTimeout;
  const timer = setTimer(() => controller.abort(), handoffMs);
  const handedOff = await requestLock(manager, name, {
    mode: 'exclusive',
    signal: controller.signal,
  });
  clearTimer(timer);
  return handedOff;
}

async function claimUniqueIdentity(win, manager, mint) {
  // A random collision is already vanishingly unlikely. Still ask the lock
  // manager, so uniqueness is established by ownership rather than probability.
  if (manager) {
    for (let attempt = 0; attempt < 4; attempt += 1) {
      const identity = mint();
      const handle = await claimIdentity(win, manager, identity, 0);
      if (handle) return { identity, handle };
    }
  }

  return null;
}

function durableIdentities(storage) {
  if (!storage) return [];
  try {
    const identities = [];
    for (let index = 0; index < storage.length; index += 1) {
      const key = storage.key(index);
      if (!key || !key.startsWith(BROWSER_SAVE_DIRECTORY_PREFIX)) continue;
      const identity = key.slice(BROWSER_SAVE_DIRECTORY_PREFIX.length);
      if (validBrowserSaveIdentity(identity)) identities.push(identity);
    }
    return identities.sort();
  } catch (_) {
    return [];
  }
}

function rememberDurableIdentity(storage, identity) {
  return safeSet(storage, BROWSER_SAVE_DIRECTORY_PREFIX + identity, identity);
}

function parseHandoff(value, sessionIdentity) {
  if (typeof value !== 'string' || !sessionIdentity) return null;
  try {
    const marker = JSON.parse(value);
    if (!marker || marker.identity !== sessionIdentity) return null;
    if (!validBrowserSaveIdentity(marker.identity)
      || !validBrowserSaveIdentity(marker.ticket)) return null;
    return marker;
  } catch (_) {
    return null;
  }
}

function coordinatorClient(win) {
  let worker;
  try {
    if (typeof win.SharedWorker !== 'function') return null;
    worker = new win.SharedWorker(COORDINATOR_URL, {
      name: COORDINATOR_NAME,
      type: 'module',
    });
  } catch (_) {
    return null;
  }

  const port = worker && worker.port;
  if (!port || typeof port.postMessage !== 'function') return null;
  if (typeof port.start === 'function') port.start();
  let sequence = 0;
  const pending = new Map();
  port.onmessage = (event) => {
    const message = event && event.data;
    const waiting = message && pending.get(message.requestId);
    if (!waiting) return;
    pending.delete(message.requestId);
    waiting.resolve(message);
  };

  function request(type, payload = {}) {
    const requestId = `${Date.now()}-${sequence += 1}`;
    return new Promise((resolve) => {
      // Claims have a worker-side bounded preference wait. Do not also abandon
      // the request on a document timer: the worker could wake later, grant the
      // stale request, and leave this connection owning a namespace the page no
      // longer knows about. A failed worker therefore stalls this pre-boot gate
      // closed instead of creating latent ownership.
      pending.set(requestId, { resolve });
      try { port.postMessage({ requestId, type, ...payload }); }
      catch (_) {
        pending.delete(requestId);
        resolve({ ok: false });
      }
    });
  }

  return {
    request,
    close() {
      try { port.close(); } catch (_) {}
    },
  };
}

async function claimWithCoordinator(win, candidates, preferred, ticket, timeoutMs) {
  const client = coordinatorClient(win);
  if (!client) return null;
  const answer = await client.request('claim', {
    candidates,
    preferred,
    ticket,
    waitMs: timeoutMs,
  });
  if (!answer.ok || !validBrowserSaveIdentity(answer.identity)) {
    client.close();
    return null;
  }
  const identity = answer.identity;
  let released = false;
  return {
    identity,
    handle: {
      kind: 'coordinator',
      async handoff(handoffTicket) {
        if (released) return false;
        const reply = await client.request('handoff', {
          identity,
          ticket: handoffTicket,
        });
        if (reply.ok) {
          released = true;
          client.close();
        }
        return !!reply.ok;
      },
      release() {
        if (released) return;
        released = true;
        void client.request('release', { identity });
      },
    },
  };
}

function publishIdentity(win, identity, sessionStore, localStore, seedShared, localReadable) {
  const sessionIdentityWritten = safeSet(sessionStore, BROWSER_SAVE_TAB_KEY, identity);
  // A durable pointer without a same-tab binding would advertise this peer's
  // catalogue to unrelated future documents while leaving its own successor
  // unable to prove which namespace belongs to it. Keep such a page in-memory
  // only and let Start fail closed.
  if (sessionIdentityWritten) {
    if (localReadable) rememberDurableIdentity(localStore, identity);
    if (seedShared) safeSet(localStore, BROWSER_SAVE_SHARED_KEY, identity);
  }
  try { win[BROWSER_SAVE_IDENTITY_PROPERTY] = identity; } catch (_) {}
  return sessionIdentityWritten;
}

/**
 * Arm the current peer identity for exactly one successor document.
 *
 * The returned promise is single-flight for this document. A Web Lock needs
 * no ticket; the ordinary pagehide/queued-claim path remains its preferred
 * atomic handoff. Every path fails closed unless the chosen identity was
 * written to sessionStorage, because the successor document cannot otherwise
 * prove which durable namespace it should enter.
 */
export function armBrowserSaveIdentityHandoff(
  win = (typeof window !== 'undefined' ? window : undefined),
) {
  if (!win || (typeof win !== 'object' && typeof win !== 'function')) {
    return Promise.resolve(false);
  }
  const state = installations.get(win);
  if (!state || !validBrowserSaveIdentity(state.identity)
    || !state.sessionIdentityWritten) return Promise.resolve(false);
  if (state.handoffPromise) return state.handoffPromise;
  if (state.handle?.kind === 'lock') {
    state.handoffPromise = Promise.resolve(true);
    return state.handoffPromise;
  }
  if (state.handle?.kind !== 'coordinator') return Promise.resolve(false);

  const operation = (async () => {
    const randomValues = (array) => win.crypto.getRandomValues(array);
    const ticket = mintToken(randomValues);
    const sessionStore = safeStorage(win, 'sessionStorage');
    const marker = JSON.stringify({ identity: state.identity, ticket });
    if (!safeSet(sessionStore, BROWSER_SAVE_HANDOFF_KEY, marker)) return false;
    if (!await state.handle.handoff(ticket)) {
      safeRemove(sessionStore, BROWSER_SAVE_HANDOFF_KEY);
      return false;
    }
    state.handle = null;
    state.handoffArmed = true;
    return true;
  })();
  state.handoffPromise = operation;
  void operation.then((ok) => {
    if (!ok && state.handoffPromise === operation) state.handoffPromise = null;
  });
  return operation;
}

/**
 * Resolve and exclusively own this browser host's save identity.
 *
 * @param {Window|object} win injectable browser-like object
 * @param {{handoffMs?: number}} options testable navigation handoff duration
 * @returns {Promise<string>} resolved before any catalogue or simulation boot
 */
export function prepareBrowserSaveIdentity(
  win = (typeof window !== 'undefined' ? window : undefined),
  { handoffMs = HANDOFF_GRACE_MS } = {},
) {
  if (!win || (typeof win !== 'object' && typeof win !== 'function')) {
    return Promise.resolve('');
  }
  const existing = installations.get(win);
  if (existing) return existing.ready;

  const state = {
    identity: '',
    handle: null,
    handoffArmed: false,
    handoffPromise: null,
    sessionIdentityWritten: false,
    suspendedForBfcache: false,
    ready: null,
  };
  const ready = (async () => {
    const randomValues = (array) => win.crypto.getRandomValues(array);
    const mint = () => mintToken(randomValues);
    const manager = lockManager(win);
    const sessionStore = safeStorage(win, 'sessionStorage');
    const localStore = safeStorage(win, 'localStorage');
    const sessionRead = safeGet(sessionStore, BROWSER_SAVE_TAB_KEY);
    const sharedRead = safeGet(localStore, BROWSER_SAVE_SHARED_KEY);
    const handoffRead = safeGet(sessionStore, BROWSER_SAVE_HANDOFF_KEY);
    const sessionIdentity = validBrowserSaveIdentity(sessionRead.value) ? sessionRead.value : null;
    const sharedIdentity = validBrowserSaveIdentity(sharedRead.value) ? sharedRead.value : null;
    const handoff = parseHandoff(handoffRead.value, sessionIdentity);
    const directory = durableIdentities(localStore);
    // A handoff is a one-shot capability. Consume even malformed/mismatched
    // values so copied sessionStorage never turns into a standing reuse route.
    if (handoffRead.readable && handoffRead.value !== null) {
      safeRemove(sessionStore, BROWSER_SAVE_HANDOFF_KEY);
    }

    let chosen = null;
    if (manager) {
      const candidates = [...new Set([
        sessionIdentity,
        sharedIdentity,
        ...directory,
      ].filter(Boolean))];
      for (const identity of candidates) {
        const handle = await claimIdentity(win, manager, identity, handoffMs);
        if (handle) {
          chosen = { identity, handle };
          break;
        }
      }
    }
    if (!chosen) chosen = await claimUniqueIdentity(win, manager, mint);
    if (!chosen) {
      const candidates = [...new Set([
        handoff?.identity,
        sessionIdentity,
        sharedIdentity,
        ...directory,
        mint(),
      ].filter(Boolean))];
      chosen = await claimWithCoordinator(
        win,
        candidates,
        sessionIdentity,
        handoff?.ticket || null,
        handoffMs,
      );
    }
    // No serialization primitive means no durable ownership claim. Keep the
    // live peer isolated under fresh entropy; Start will fail closed because
    // `armBrowserSaveIdentityHandoff` has no transferable handle.
    if (!chosen) chosen = { identity: mint(), handle: null };

    state.identity = chosen.identity;
    state.handle = chosen.handle;
    // Seed/repair the durable single-host pointer only when its read succeeded
    // and yielded no valid identity. A failed getItem never authorises a write
    // that might replace another peer's pointer.
    const seedShared = sharedRead.readable && !sharedIdentity;
    state.sessionIdentityWritten = publishIdentity(
      win,
      state.identity,
      sessionStore,
      localStore,
      seedShared,
      sharedRead.readable,
    );

    return state.identity;
  })();
  state.ready = ready;
  installations.set(win, state);

  // Release on navigation/close. A new same-tab document queues briefly for
  // this exact handoff. No TTL is involved, so sleep/throttling cannot make a
  // live identity appear abandoned.
  if (typeof win.addEventListener === 'function') {
    win.addEventListener('pagehide', (event) => {
      // A cached Phoenix App must never resume past the pre-boot ownership
      // gate. Release either primitive while the document is suspended; its
      // persisted pageshow below reloads instead of resuming this App.
      if (event?.persisted) state.suspendedForBfcache = true;
      if (state.handoffArmed) return;
      state.handle?.release();
      state.handle = null;
    });
    win.addEventListener('pageshow', (event) => {
      if (!event || !event.persisted || !state.suspendedForBfcache) return;
      // Neither a released Web Lock nor a handed-off/retained coordinator
      // claim can fence a cached WASM instance atomically on Back. Reload
      // synchronously from Phoenix's earliest pageshow listener and re-enter
      // the ordinary pre-boot gate before any catalogue or simulation work.
      try { win.location.reload(); } catch (_) {}
    });
  }

  return ready;
}

// Backwards-friendly name for call sites/tests that only care about the value.
export const installBrowserSaveIdentity = prepareBrowserSaveIdentity;
