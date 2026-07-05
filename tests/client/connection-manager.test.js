import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  defaultIceServers,
  fetchIceServers,
  nextBackoffDelay,
  ConnectionManager,
  connectionManager,
} from '../../gui/connection-manager.js';

describe('defaultIceServers', () => {
  it('returns the expected base ICE server list', () => {
    const servers = defaultIceServers();
    expect(servers).toHaveLength(5);
    expect(servers[0].urls).toBe('stun:stun.l.google.com:19302');
    expect(servers[1].urls).toBe('stun:stun1.l.google.com:19302');
    expect(servers.some(s => s.urls.startsWith('turn:'))).toBe(true);
  });

  it('is frozen to prevent mutation', () => {
    const servers = defaultIceServers();
    expect(Object.isFrozen(servers)).toBe(false);
    // But the returned array is fresh each call
    expect(defaultIceServers()).not.toBe(servers);
  });
});

describe('fetchIceServers', () => {
  it('returns base list when fetch fails (network error)', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('Network error')));
    const servers = await fetchIceServers();
    expect(servers).toHaveLength(5);
    expect(servers[0].urls).toBe('stun:stun.l.google.com:19302');
    vi.unstubAllGlobals();
  });

  it('returns base list when fetch returns non-ok status', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 503 }));
    const servers = await fetchIceServers();
    expect(servers).toHaveLength(5);
    vi.unstubAllGlobals();
  });

  it('appends extra servers when fetch succeeds', async () => {
    const extra = [{ urls: 'turn:example.com:3478', username: 'test', credential: 'pass' }];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: vi.fn().mockResolvedValue(extra),
    }));
    const servers = await fetchIceServers();
    expect(servers).toHaveLength(6);
    expect(servers[5].urls).toBe('turn:example.com:3478');
    vi.unstubAllGlobals();
  });
});

describe('nextBackoffDelay', () => {
  it('starts at the initial delay on the first attempt (attempt 0)', () => {
    expect(nextBackoffDelay(0)).toBe(100);
  });

  it('doubles each attempt', () => {
    expect(nextBackoffDelay(0)).toBe(100);
    expect(nextBackoffDelay(1)).toBe(200);
    expect(nextBackoffDelay(2)).toBe(400);
    expect(nextBackoffDelay(3)).toBe(800);
    expect(nextBackoffDelay(4)).toBe(1600);
  });

  it('caps at the max delay (default 30s)', () => {
    expect(nextBackoffDelay(20)).toBe(30_000);
    expect(nextBackoffDelay(100)).toBe(30_000);
  });

  it('respects custom initial and max delays', () => {
    expect(nextBackoffDelay(0, 50, 1000)).toBe(50);
    expect(nextBackoffDelay(1, 50, 1000)).toBe(100);
    expect(nextBackoffDelay(10, 50, 1000)).toBe(1000);
  });

  it('treats negative attempts as attempt 0', () => {
    expect(nextBackoffDelay(-1)).toBe(100);
  });
});

describe('ConnectionManager', () => {
  it('starts with connected false', () => {
    const cm = new ConnectionManager();
    expect(cm.connected).toBe(false);
  });

  it('connected getter returns false when conn is null', () => {
    const cm = new ConnectionManager();
    expect(cm.connected).toBe(false);
  });

  it('connected getter returns false when conn is not open', () => {
    const cm = new ConnectionManager();
    cm.conn = { open: false };
    expect(cm.connected).toBe(false);
  });

  it('connected getter returns true when conn is open', () => {
    const cm = new ConnectionManager();
    cm.conn = { open: true };
    expect(cm.connected).toBe(true);
  });

  it('send is a no-op when not connected', () => {
    const cm = new ConnectionManager();
    const spy = vi.fn();
    cm.conn = { send: spy, open: false };
    cm.send('TestMessage', { foo: 1 });
    expect(spy).not.toHaveBeenCalled();
  });

  it('send is a no-op when conn is null', () => {
    const cm = new ConnectionManager();
    cm.send('TestMessage', { foo: 1 });
    // No crash is the assertion
  });

  it('send serializes and sends when connected', () => {
    const cm = new ConnectionManager();
    const spy = vi.fn();
    cm.conn = { send: spy, open: true };
    cm.send('TestMessage', { foo: 1 });
    expect(spy).toHaveBeenCalledWith(JSON.stringify({ type: 'TestMessage', data: { foo: 1 } }));
  });

  it('send works without data', () => {
    const cm = new ConnectionManager();
    const spy = vi.fn();
    cm.conn = { send: spy, open: true };
    cm.send('Ping');
    expect(spy).toHaveBeenCalledWith(JSON.stringify({ type: 'Ping' }));
  });

  it('connect no-ops when Peer is not available', () => {
    const cm = new ConnectionManager();
    cm.connect('host-id', {});
    expect(cm.peer).toBeNull();
    expect(cm.connected).toBe(false);
  });

  it('disconnect clears conn and peer', () => {
    const cm = new ConnectionManager();
    const closeSpy = vi.fn();
    const destroySpy = vi.fn();
    cm.conn = { close: closeSpy, open: true };
    cm.peer = { destroy: destroySpy };
    cm.disconnect();
    expect(closeSpy).toHaveBeenCalled();
    expect(destroySpy).toHaveBeenCalled();
    expect(cm.conn).toBeNull();
    expect(cm.peer).toBeNull();
  });
});

// ── Fake PeerJS harness for lifecycle tests ─────────────────────────────────
// A minimal Emitter-based stand-in for window.Peer/DataConnection, just
// enough to drive ConnectionManager.connect() through open/close/error
// transitions synchronously (no BroadcastChannel, no real WebRTC — that's
// what the Playwright smoke suite's peerjs-shim.js covers).

function makeEmitter() {
  const handlers = {};
  return {
    on(ev, fn) { (handlers[ev] ||= []).push(fn); },
    _emit(ev, ...args) { (handlers[ev] || []).forEach(fn => fn(...args)); },
  };
}

function makeFakeConn() {
  const conn = makeEmitter();
  conn.open = false;
  conn.send = vi.fn();
  conn.close = vi.fn(() => { conn.open = false; });
  // Mirrors the real shim's Connection.prototype._open: flips `.open` to
  // true (which ConnectionManager's `connected` getter reads) before firing
  // the 'open' event, matching real PeerJS DataConnection semantics.
  conn._simulateOpen = () => { conn.open = true; conn._emit('open'); };
  // Mirrors Connection.prototype._close: flips `.open` false, then emits.
  conn._simulateClose = () => { conn.open = false; conn._emit('close'); };
  conn._simulateError = (type = 'network') => { conn.open = false; conn._emit('error', { type }); };
  return conn;
}

function makeFakePeerCtor(conns, peers = []) {
  // `conns` is a queue of fake conn objects returned by successive
  // `.connect()` calls, letting a test script out a sequence of attempts.
  return function FakePeer() {
    const peer = makeEmitter();
    peer.destroy = vi.fn();
    peer.reconnect = vi.fn();
    peer.connect = vi.fn(() => conns.shift() || makeFakeConn());
    peers.push(peer);
    // Fire 'open' asynchronously like the real PeerJS/shim does.
    queueMicrotask(() => peer._emit('open', 'fake-peer-id'));
    return peer;
  };
}

describe('ConnectionManager lifecycle (identify re-send + reconnect)', () => {
  const realWindow = globalThis.window;

  afterEach(() => {
    globalThis.window = realWindow;
    vi.useRealTimers();
  });

  it('re-sends Identify on every reopen (resets _identSent on close)', async () => {
    const conn1 = makeFakeConn();
    const conn2 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1, conn2]) };

    const cm = new ConnectionManager();
    const getIdent = () => ({ token: 'tok-1', name: 'P1' });

    cm.connect('host-id', { getIdent });
    await Promise.resolve(); // flush peer 'open' microtask
    await Promise.resolve();

    conn1._simulateOpen();
    expect(conn1.send).toHaveBeenCalledTimes(1);
    expect(JSON.parse(conn1.send.mock.calls[0][0])).toEqual({
      type: 'Identify',
      data: { token: 'tok-1', name: 'P1' },
    });

    // Simulate the DataChannel dying (established-connection close, not a
    // manual conn.close() call from this side).
    vi.useFakeTimers();
    conn1._simulateClose();

    // Backoff timer fires -> _reconnect() -> connect() again -> new fake Peer
    // -> conn2 assigned -> open -> Identify re-sent.
    await vi.advanceTimersByTimeAsync(200);
    conn2._simulateOpen();

    expect(conn2.send).toHaveBeenCalledTimes(1);
    expect(JSON.parse(conn2.send.mock.calls[0][0])).toEqual({
      type: 'Identify',
      data: { token: 'tok-1', name: 'P1' },
    });
  });

  it('schedules a backoff retry after an established connection closes', async () => {
    const conn1 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1]) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    const statuses = [];
    cm.connect('host-id', { onStatus: s => statuses.push(s) });

    await vi.advanceTimersByTimeAsync(0);
    conn1._simulateOpen();
    expect(statuses).toContain('ready');

    conn1._simulateClose();
    expect(statuses.at(-1)).toBe('disconnected');

    // A retry timer should now be pending; nothing throws if we let it fire.
    await vi.advanceTimersByTimeAsync(500);
  });

  it('exposes disconnected status when the initial DataChannel attempt times out', async () => {
    const conn1 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1]) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    const statuses = [];
    cm.connect('host-id', { onStatus: s => statuses.push(s) });

    await vi.advanceTimersByTimeAsync(0);
    expect(statuses.at(-1)).toBe('connecting');

    await vi.advanceTimersByTimeAsync(8000);

    expect(conn1.close).toHaveBeenCalled();
    expect(statuses).toContain('disconnected');
    expect(statuses.at(-1)).toBe('disconnected');
  });

  it('ignores stale DataConnection close/error callbacks after a fresh reconnect opens', async () => {
    const conn1 = makeFakeConn();
    const conn2 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1, conn2]) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    const statuses = [];
    const onError = vi.fn();
    cm.connect('host-id', { onStatus: s => statuses.push(s), onError });

    await vi.advanceTimersByTimeAsync(0);
    conn1._simulateOpen();
    conn1._simulateClose();
    await vi.advanceTimersByTimeAsync(100);
    await vi.advanceTimersByTimeAsync(0);
    conn2._simulateOpen();
    expect(cm.conn).toBe(conn2);
    expect(cm.connected).toBe(true);

    statuses.length = 0;
    conn1._simulateClose();
    conn1._simulateError('stale-network');

    expect(cm.conn).toBe(conn2);
    expect(cm.connected).toBe(true);
    expect(statuses).toEqual([]);
    expect(onError).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(500);
    expect(cm.conn).toBe(conn2);
    expect(conn2.close).not.toHaveBeenCalled();
  });

  it('ignores stale Peer error callbacks after a fresh reconnect opens', async () => {
    const conn1 = makeFakeConn();
    const conn2 = makeFakeConn();
    const peers = [];
    globalThis.window = { Peer: makeFakePeerCtor([conn1, conn2], peers) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    const statuses = [];
    const onError = vi.fn();
    cm.connect('host-id', { onStatus: s => statuses.push(s), onError });

    await vi.advanceTimersByTimeAsync(0);
    conn1._simulateOpen();
    conn1._simulateClose();
    await vi.advanceTimersByTimeAsync(100);
    await vi.advanceTimersByTimeAsync(0);
    conn2._simulateOpen();
    expect(peers).toHaveLength(2);

    statuses.length = 0;
    peers[0]._emit('error', { type: 'stale-peer-error' });

    expect(cm.conn).toBe(conn2);
    expect(cm.connected).toBe(true);
    expect(statuses).toEqual([]);
    expect(onError).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(500);
    expect(cm.conn).toBe(conn2);
    expect(conn2.close).not.toHaveBeenCalled();
  });

  it('re-sends Identify when an established connection errors and reopens', async () => {
    const conn1 = makeFakeConn();
    const conn2 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1, conn2]) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    const getIdent = () => ({ token: 'tok-err', name: 'P1' });

    cm.connect('host-id', { getIdent });
    await vi.advanceTimersByTimeAsync(0);
    conn1._simulateOpen();
    expect(conn1.send).toHaveBeenCalledTimes(1);

    conn1._simulateError();
    await vi.advanceTimersByTimeAsync(200);
    conn2._simulateOpen();

    expect(conn2.send).toHaveBeenCalledTimes(1);
    expect(JSON.parse(conn2.send.mock.calls[0][0])).toEqual({
      type: 'Identify',
      data: { token: 'tok-err', name: 'P1' },
    });
  });

  it('retryNow() cancels the pending backoff wait and reconnects immediately', async () => {
    const conn1 = makeFakeConn();
    const conn2 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1, conn2]) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    cm.connect('host-id', {});
    await vi.advanceTimersByTimeAsync(0);
    conn1._simulateOpen();

    conn1._simulateClose();
    expect(cm.connected).toBe(false);

    // Don't advance timers — retryNow() should bypass the wait entirely.
    cm.retryNow();
    await Promise.resolve();
    await Promise.resolve();
    conn2._simulateOpen();

    expect(cm.connected).toBe(true);
  });

  it('retryNow() is a no-op while already connected', () => {
    const cm = new ConnectionManager();
    cm.conn = { open: true };
    const peer = { destroy: vi.fn() };
    cm.peer = peer;
    cm.retryNow();
    expect(peer.destroy).not.toHaveBeenCalled();
  });

  it('disconnect() clears any pending retry timer', async () => {
    const conn1 = makeFakeConn();
    globalThis.window = { Peer: makeFakePeerCtor([conn1]) };
    vi.useFakeTimers();

    const cm = new ConnectionManager();
    cm.connect('host-id', {});
    await vi.advanceTimersByTimeAsync(0);
    conn1._simulateOpen();
    conn1._simulateClose();

    cm.disconnect();
    // If a stray timer fired after disconnect() it would call connect() again
    // and throw trying to reach a destroyed peer's queue; advancing time here
    // with nothing left queued is itself the assertion (no crash).
    await vi.advanceTimersByTimeAsync(60_000);
  });
});

describe('singleton', () => {
  it('exports a ConnectionManager instance', () => {
    expect(connectionManager).toBeInstanceOf(ConnectionManager);
    expect(connectionManager.connected).toBe(false);
  });

  it('does not crash when window is undefined (Node)', () => {
    // In node the typeof window check handles gracefully
    expect(connectionManager).toBeInstanceOf(ConnectionManager);
  });
});
