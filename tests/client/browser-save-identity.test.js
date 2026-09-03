import { describe, expect, it } from 'vitest';
import {
  armBrowserSaveIdentityHandoff,
  BROWSER_SAVE_HANDOFF_KEY,
  BROWSER_SAVE_IDENTITY_PROPERTY,
  BROWSER_SAVE_LOCK_PREFIX,
  BROWSER_SAVE_SHARED_KEY,
  BROWSER_SAVE_TAB_KEY,
  prepareBrowserSaveIdentity,
  validBrowserSaveIdentity,
} from '../../gui/browser-save-identity.js';
import { BrowserSaveIdentityCoordinator } from '../../gui/browser-save-identity-worker.js';

class MemoryStorage {
  constructor(entries = []) {
    this.values = new Map(entries);
  }

  getItem(key) {
    return this.values.has(key) ? this.values.get(key) : null;
  }

  setItem(key, value) {
    this.values.set(String(key), String(value));
  }

  removeItem(key) {
    this.values.delete(String(key));
  }

  get length() {
    return this.values.size;
  }

  key(index) {
    return Array.from(this.values.keys())[index] ?? null;
  }
}

class FakeMessagePort {
  constructor() {
    this.peer = null;
    this.onmessage = null;
  }

  postMessage(data) {
    Promise.resolve().then(() => this.peer?.onmessage?.({ data }));
  }

  start() {}

  close() {}
}

class FakeSharedWorker {
  static coordinator = new BrowserSaveIdentityCoordinator();

  static reset() {
    FakeSharedWorker.coordinator = new BrowserSaveIdentityCoordinator();
  }

  constructor() {
    const client = new FakeMessagePort();
    const worker = new FakeMessagePort();
    client.peer = worker;
    worker.peer = client;
    this.port = client;
    FakeSharedWorker.coordinator.connect(worker);
  }
}

class FakeLockManager {
  constructor() {
    this.held = new Set();
    this.queues = new Map();
  }

  isHeld(name) {
    return this.held.has(name);
  }

  request(name, options = {}, callback) {
    return new Promise((resolve, reject) => {
      const entry = { name, options, callback, resolve, reject, abort: null };
      if (options.signal?.aborted) {
        reject(new DOMException('aborted', 'AbortError'));
        return;
      }

      if (options.ifAvailable) {
        if (this.held.has(name)) {
          Promise.resolve().then(() => callback(null)).then(resolve, reject);
        } else {
          this.#grant(entry);
        }
        return;
      }

      if (!this.held.has(name)) {
        this.#grant(entry);
        return;
      }

      const queue = this.queues.get(name) || [];
      queue.push(entry);
      this.queues.set(name, queue);
      if (options.signal) {
        entry.abort = () => {
          const waiting = this.queues.get(name) || [];
          const index = waiting.indexOf(entry);
          if (index >= 0) waiting.splice(index, 1);
          reject(new DOMException('aborted', 'AbortError'));
        };
        options.signal.addEventListener('abort', entry.abort, { once: true });
      }
    });
  }

  #grant(entry) {
    const { name, options, callback, resolve, reject } = entry;
    if (entry.abort && options.signal) {
      options.signal.removeEventListener('abort', entry.abort);
    }
    this.held.add(name);
    Promise.resolve()
      .then(() => callback({ name, mode: 'exclusive' }))
      .then(
        (value) => { this.#release(name); resolve(value); },
        (error) => { this.#release(name); reject(error); },
      );
  }

  #release(name) {
    this.held.delete(name);
    const queue = this.queues.get(name) || [];
    const next = queue.shift();
    if (queue.length === 0) this.queues.delete(name);
    if (next) this.#grant(next);
  }
}

let entropy = 1;
function fakeWindow({
  localStorage = new MemoryStorage(),
  sessionStorage = new MemoryStorage(),
  locks = new FakeLockManager(),
  SharedWorker = FakeSharedWorker,
} = {}) {
  const listeners = new Map();
  let reloads = 0;
  return {
    localStorage,
    sessionStorage,
    navigator: { locks },
    SharedWorker,
    AbortController,
    DOMException,
    crypto: {
      getRandomValues(bytes) {
        bytes.fill(entropy++);
        return bytes;
      },
    },
    setTimeout,
    clearTimeout,
    location: { reload() { reloads += 1; } },
    reloadCount() { return reloads; },
    addEventListener(type, listener) { listeners.set(type, listener); },
    fire(type, event = {}) { listeners.get(type)?.(event); },
  };
}

const identity = (digit) => digit.repeat(32);
const nextTask = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('browser save peer identity', () => {
  it('accepts only the canonical identity shape the Rust namespace gate accepts', () => {
    expect(validBrowserSaveIdentity('0123456789abcdef0123456789abcdef')).toBe(true);
    expect(validBrowserSaveIdentity('0123456789ABCDEF0123456789ABCDEF')).toBe(false);
    expect(validBrowserSaveIdentity('0123456789abcdef:123456789abcdef')).toBe(false);
    expect(validBrowserSaveIdentity('short')).toBe(false);
  });

  it('expires a worker preference without granting that port a second identity later', async () => {
    const coordinator = new BrowserSaveIdentityCoordinator();
    const owner = { messages: [], postMessage(message) { this.messages.push(message); } };
    const waiter = { messages: [], postMessage(message) { this.messages.push(message); } };
    const preferred = identity('1');
    const alternate = identity('2');
    coordinator.receive(owner, {
      requestId: 'owner', type: 'claim', candidates: [preferred], waitMs: 0,
    });
    coordinator.receive(waiter, {
      requestId: 'waiter',
      type: 'claim',
      preferred,
      candidates: [preferred, alternate],
      waitMs: 5,
    });

    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(coordinator.owners.get(alternate)).toBe(waiter);
    coordinator.receive(owner, {
      requestId: 'release', type: 'release', identity: preferred,
    });
    expect(coordinator.owners.get(preferred)).not.toBe(waiter);
    expect(Array.from(coordinator.owners.values()).filter((port) => port === waiter)).toHaveLength(1);
  });

  it('atomically separates simultaneous installs contending for one durable identity', async () => {
    entropy = 1;
    const shared = identity('a');
    const local = new MemoryStorage([[BROWSER_SAVE_SHARED_KEY, shared]]);
    const locks = new FakeLockManager();
    const first = fakeWindow({ localStorage: local, locks });
    const second = fakeWindow({ localStorage: local, locks });

    const [firstIdentity, secondIdentity] = await Promise.all([
      prepareBrowserSaveIdentity(first, { handoffMs: 5 }),
      prepareBrowserSaveIdentity(second, { handoffMs: 5 }),
    ]);

    expect(firstIdentity).toBe(shared);
    expect(secondIdentity).not.toBe(shared);
    expect(secondIdentity).not.toBe(firstIdentity);
    expect(local.getItem(BROWSER_SAVE_SHARED_KEY)).toBe(shared);
    expect(locks.isHeld(BROWSER_SAVE_LOCK_PREFIX + firstIdentity)).toBe(true);
    expect(locks.isHeld(BROWSER_SAVE_LOCK_PREFIX + secondIdentity)).toBe(true);
  });

  it('separates a duplicated context that cloned both persistent identity values', async () => {
    entropy = 10;
    const cloned = identity('b');
    const local = new MemoryStorage([[BROWSER_SAVE_SHARED_KEY, cloned]]);
    const locks = new FakeLockManager();
    const first = fakeWindow({
      localStorage: local,
      sessionStorage: new MemoryStorage([[BROWSER_SAVE_TAB_KEY, cloned]]),
      locks,
    });
    const duplicate = fakeWindow({
      localStorage: local,
      sessionStorage: new MemoryStorage([[BROWSER_SAVE_TAB_KEY, cloned]]),
      locks,
    });

    expect(await prepareBrowserSaveIdentity(first, { handoffMs: 5 })).toBe(cloned);
    const duplicateIdentity = await prepareBrowserSaveIdentity(duplicate, { handoffMs: 5 });
    expect(duplicateIdentity).not.toBe(cloned);
    expect(duplicate[BROWSER_SAVE_IDENTITY_PROPERTY]).toBe(duplicateIdentity);
  });

  it('queues briefly for same-tab navigation handoff without depending on pagehide timing', async () => {
    entropy = 20;
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const locks = new FakeLockManager();
    const before = fakeWindow({ localStorage: local, sessionStorage: session, locks });
    const original = await prepareBrowserSaveIdentity(before, { handoffMs: 100 });
    const after = fakeWindow({ localStorage: local, sessionStorage: session, locks });

    const pending = prepareBrowserSaveIdentity(after, { handoffMs: 100 });
    setTimeout(() => before.fire('pagehide'), 5);
    expect(await pending).toBe(original);
    expect(locks.isHeld(BROWSER_SAVE_LOCK_PREFIX + original)).toBe(true);
  });

  it('falls back to a unique identity when a live owner misses pagehide', async () => {
    entropy = 30;
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const locks = new FakeLockManager();
    const before = fakeWindow({ localStorage: local, sessionStorage: session, locks });
    const original = await prepareBrowserSaveIdentity(before, { handoffMs: 5 });
    const after = fakeWindow({ localStorage: local, sessionStorage: session, locks });

    const fallback = await prepareBrowserSaveIdentity(after, { handoffMs: 5 });
    expect(fallback).not.toBe(original);
    expect(locks.isHeld(BROWSER_SAVE_LOCK_PREFIX + original)).toBe(true);
    expect(locks.isHeld(BROWSER_SAVE_LOCK_PREFIX + fallback)).toBe(true);
  });

  it('reloads synchronously before a Web-Lock BFCache document can resume', async () => {
    entropy = 40;
    const locks = new FakeLockManager();
    const win = fakeWindow({ locks });
    const selected = await prepareBrowserSaveIdentity(win, { handoffMs: 20 });
    const lockName = BROWSER_SAVE_LOCK_PREFIX + selected;
    expect(locks.isHeld(lockName)).toBe(true);

    win.fire('pagehide', { persisted: true });
    await nextTask();
    expect(locks.isHeld(lockName)).toBe(false);
    win.fire('pageshow', { persisted: true });
    expect(win.reloadCount()).toBe(1);
    expect(locks.isHeld(lockName)).toBe(false);
  });

  it('reloads instead of sharing when a BFCache identity was claimed while suspended', async () => {
    entropy = 45;
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const locks = new FakeLockManager();
    const cached = fakeWindow({ localStorage: local, sessionStorage: session, locks });
    const selected = await prepareBrowserSaveIdentity(cached, { handoffMs: 5 });
    cached.fire('pagehide', { persisted: true });
    await nextTask();

    const other = fakeWindow({ localStorage: local, sessionStorage: session, locks });
    expect(await prepareBrowserSaveIdentity(other, { handoffMs: 5 })).toBe(selected);
    cached.fire('pageshow', { persisted: true });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(cached.reloadCount()).toBe(1);
  });

  it('keeps unavailable-lock peers private while handing each catalogue to its successor', async () => {
    entropy = 50;
    const persisted = identity('c');
    const local = new MemoryStorage([[BROWSER_SAVE_SHARED_KEY, persisted]]);
    const firstSession = new MemoryStorage();
    const secondSession = new MemoryStorage();
    const first = fakeWindow({ localStorage: local, sessionStorage: firstSession, locks: null });
    const second = fakeWindow({ localStorage: local, sessionStorage: secondSession, locks: null });
    first.navigator = {};
    second.navigator = {};

    const firstIdentity = await prepareBrowserSaveIdentity(first, { handoffMs: 0 });
    const secondIdentity = await prepareBrowserSaveIdentity(second, { handoffMs: 0 });
    expect(firstIdentity).toBe(persisted);
    expect(secondIdentity).not.toBe(persisted);
    expect(secondIdentity).not.toBe(firstIdentity);

    expect(await armBrowserSaveIdentityHandoff(first)).toBe(true);
    expect(await armBrowserSaveIdentityHandoff(second)).toBe(true);
    expect(JSON.parse(firstSession.getItem(BROWSER_SAVE_HANDOFF_KEY)).identity)
      .toBe(firstIdentity);
    expect(JSON.parse(secondSession.getItem(BROWSER_SAVE_HANDOFF_KEY)).identity)
      .toBe(secondIdentity);

    const firstSuccessor = fakeWindow({
      localStorage: local,
      sessionStorage: firstSession,
      locks: null,
    });
    const secondSuccessor = fakeWindow({
      localStorage: local,
      sessionStorage: secondSession,
      locks: null,
    });
    firstSuccessor.navigator = {};
    secondSuccessor.navigator = {};

    expect(await prepareBrowserSaveIdentity(firstSuccessor, { handoffMs: 0 }))
      .toBe(firstIdentity);
    expect(await prepareBrowserSaveIdentity(secondSuccessor, { handoffMs: 0 }))
      .toBe(secondIdentity);
    expect(firstSession.getItem(BROWSER_SAVE_HANDOFF_KEY)).toBeNull();
    expect(secondSession.getItem(BROWSER_SAVE_HANDOFF_KEY)).toBeNull();
  });

  it('preferentially reclaims a released session identity on an ordinary SharedWorker reload', async () => {
    entropy = 53;
    FakeSharedWorker.reset();
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const before = fakeWindow({ localStorage: local, sessionStorage: session, locks: null });
    before.navigator = {};
    const original = await prepareBrowserSaveIdentity(before, { handoffMs: 50 });

    const after = fakeWindow({ localStorage: local, sessionStorage: session, locks: null });
    after.navigator = {};
    const pending = prepareBrowserSaveIdentity(after, { handoffMs: 50 });
    setTimeout(() => before.fire('pagehide', { persisted: false }), 5);

    expect(await pending).toBe(original);
    expect(after[BROWSER_SAVE_IDENTITY_PROPERTY]).toBe(original);
  });

  it('reloads a Start predecessor restored from BFCache after its successor', async () => {
    entropy = 54;
    FakeSharedWorker.reset();
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const before = fakeWindow({ localStorage: local, sessionStorage: session, locks: null });
    before.navigator = {};
    const original = await prepareBrowserSaveIdentity(before, { handoffMs: 20 });
    expect(await armBrowserSaveIdentityHandoff(before)).toBe(true);
    before.fire('pagehide', { persisted: true });

    const successor = fakeWindow({ localStorage: local, sessionStorage: session, locks: null });
    successor.navigator = {};
    expect(await prepareBrowserSaveIdentity(successor, { handoffMs: 20 })).toBe(original);
    successor.fire('pagehide', { persisted: true });
    await nextTask();

    before.fire('pageshow', { persisted: true });
    expect(before.reloadCount()).toBe(1);
  });

  it('does not treat cloned sessionStorage as a fallback handoff', async () => {
    entropy = 55;
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const owner = fakeWindow({ localStorage: local, sessionStorage: session, locks: null });
    owner.navigator = {};
    const ownerIdentity = await prepareBrowserSaveIdentity(owner, { handoffMs: 0 });
    const clonedSession = new MemoryStorage(session.values);
    const duplicate = fakeWindow({
      localStorage: local,
      sessionStorage: clonedSession,
      locks: null,
    });
    duplicate.navigator = {};

    const duplicateIdentity = await prepareBrowserSaveIdentity(duplicate, { handoffMs: 0 });
    expect(duplicateIdentity).not.toBe(ownerIdentity);
  });

  it('uses the same one-shot handoff when a LockManager denies every request', async () => {
    entropy = 57;
    const denied = { request() { throw new DOMException('denied', 'SecurityError'); } };
    const local = new MemoryStorage();
    const session = new MemoryStorage();
    const before = fakeWindow({ localStorage: local, sessionStorage: session, locks: denied });
    const selected = await prepareBrowserSaveIdentity(before, { handoffMs: 0 });
    expect(await armBrowserSaveIdentityHandoff(before)).toBe(true);

    const after = fakeWindow({ localStorage: local, sessionStorage: session, locks: denied });
    expect(await prepareBrowserSaveIdentity(after, { handoffMs: 0 })).toBe(selected);
  });

  it('arms a SharedWorker handoff as one single-flight ticket', async () => {
    entropy = 57;
    FakeSharedWorker.reset();
    const session = new MemoryStorage();
    const win = fakeWindow({ sessionStorage: session, locks: null });
    win.navigator = {};
    const selected = await prepareBrowserSaveIdentity(win, { handoffMs: 0 });

    const first = armBrowserSaveIdentityHandoff(win);
    const second = armBrowserSaveIdentityHandoff(win);
    expect(second).toBe(first);
    expect(await Promise.all([first, second])).toEqual([true, true]);

    const marker = JSON.parse(session.getItem(BROWSER_SAVE_HANDOFF_KEY));
    expect(marker.identity).toBe(selected);
    expect(Array.from(FakeSharedWorker.coordinator.reservations.entries()))
      .toEqual([[marker.ticket, selected]]);
  });

  it('recovers every closed peer namespace exactly once through a fresh coordinator', async () => {
    entropy = 58;
    FakeSharedWorker.reset();
    const local = new MemoryStorage();
    const first = fakeWindow({ localStorage: local, sessionStorage: new MemoryStorage(), locks: null });
    const second = fakeWindow({ localStorage: local, sessionStorage: new MemoryStorage(), locks: null });
    first.navigator = {};
    second.navigator = {};
    const original = new Set(await Promise.all([
      prepareBrowserSaveIdentity(first, { handoffMs: 20 }),
      prepareBrowserSaveIdentity(second, { handoffMs: 20 }),
    ]));
    expect(original.size).toBe(2);

    first.fire('pagehide', { persisted: false });
    second.fire('pagehide', { persisted: false });
    await nextTask();

    // All documents closed: the browser terminates the old worker. A later
    // browser session gets a fresh serialization point over the durable keys.
    FakeSharedWorker.reset();
    const recoveredFirst = fakeWindow({
      localStorage: local,
      sessionStorage: new MemoryStorage(),
      locks: null,
    });
    const recoveredSecond = fakeWindow({
      localStorage: local,
      sessionStorage: new MemoryStorage(),
      locks: null,
    });
    recoveredFirst.navigator = {};
    recoveredSecond.navigator = {};
    const recovered = new Set(await Promise.all([
      prepareBrowserSaveIdentity(recoveredFirst, { handoffMs: 20 }),
      prepareBrowserSaveIdentity(recoveredSecond, { handoffMs: 20 }),
    ]));

    expect(recovered).toEqual(original);
  });

  it('fails closed when neither Web Locks nor SharedWorker can serialize Start', async () => {
    entropy = 59;
    const win = fakeWindow({ locks: null, SharedWorker: null });
    win.navigator = {};
    const selected = await prepareBrowserSaveIdentity(win, { handoffMs: 0 });
    expect(selected).toMatch(/^[0-9a-f]{32}$/);
    expect(await armBrowserSaveIdentityHandoff(win)).toBe(false);
  });

  it('fails Start closed when the selected identity cannot reach sessionStorage', async () => {
    entropy = 59;
    const brokenSession = {
      getItem() { return null; },
      setItem() { throw new DOMException('write denied', 'QuotaExceededError'); },
      removeItem() {},
    };
    const locks = new FakeLockManager();
    const local = new MemoryStorage();
    const win = fakeWindow({ localStorage: local, sessionStorage: brokenSession, locks });
    const selected = await prepareBrowserSaveIdentity(win, { handoffMs: 5 });

    expect(locks.isHeld(BROWSER_SAVE_LOCK_PREFIX + selected)).toBe(true);
    expect(await armBrowserSaveIdentityHandoff(win)).toBe(false);
    expect(local.length).toBe(0);
  });

  it('contains per-method storage failures without trusting or overwriting unread state', async () => {
    entropy = 60;
    const localWrites = [];
    const brokenLocal = {
      getItem() { throw new DOMException('read denied', 'SecurityError'); },
      setItem(key, value) { localWrites.push([key, value]); },
    };
    const brokenSession = {
      getItem() { throw new DOMException('read denied', 'SecurityError'); },
      setItem() { throw new DOMException('write denied', 'QuotaExceededError'); },
    };
    const win = fakeWindow({ localStorage: brokenLocal, sessionStorage: brokenSession });
    const selected = await prepareBrowserSaveIdentity(win, { handoffMs: 5 });

    expect(selected).toMatch(/^[0-9a-f]{32}$/);
    expect(win[BROWSER_SAVE_IDENTITY_PROPERTY]).toBe(selected);
    expect(localWrites).toEqual([]);
  });
});
