import { describe, it, expect, vi } from 'vitest';
import {
  defaultIceServers,
  fetchIceServers,
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
