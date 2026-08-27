// gui/connection-manager.js — ICE/TURN configuration and connection timing
// policy. Everything under test here is pure: the STUN/TURN server list and the
// credential-worker fallback ladder, the relay reachability probe, and the two
// schedules (connect timeout, reconnect backoff) the Phoenix transport is tuned
// by.
//
// The connection LIFECYCLE tests that used to live here — identify re-send,
// backoff after an established close, timeout escalation, the snapshot channel
// — went with `class ConnectionManager` when issue #1112 retired PeerJS. Their
// successors assert the same behaviours against the shipped transport in
// tests/client/rendezvous-transport.test.js, driven through the real rendezvous
// registry rather than a hand-rolled fake PeerJS emitter.

import { describe, it, expect, vi } from 'vitest';
import {
  defaultIceServers,
  openRelayFallbackServers,
  fetchIceServers,
  hasRelayServer,
  nextBackoffDelay,
  connectTimeoutMs,
  probeTurnRelay,
  candidateType,
} from '../../gui/connection-manager.js';

describe('defaultIceServers', () => {
  it('returns the STUN-only base list (dead OpenRelay TURN entries removed)', () => {
    const servers = defaultIceServers();
    expect(servers).toHaveLength(2);
    expect(servers[0].urls).toBe('stun:stun.l.google.com:19302');
    expect(servers[1].urls).toBe('stun:stun1.l.google.com:19302');
    expect(servers.some(s => s.urls.startsWith('turn:'))).toBe(false);
  });

  it('is frozen to prevent mutation', () => {
    const servers = defaultIceServers();
    expect(Object.isFrozen(servers)).toBe(false);
    // But the returned array is fresh each call
    expect(defaultIceServers()).not.toBe(servers);
  });
});

describe('hasRelayServer', () => {
  it('is false for STUN-only lists', () => {
    expect(hasRelayServer(defaultIceServers())).toBe(false);
  });

  it('detects turn: and turns: urls, including url arrays', () => {
    expect(hasRelayServer([{ urls: 'turn:r.example.com:80' }])).toBe(true);
    expect(hasRelayServer([{ urls: 'turns:r.example.com:443?transport=tcp' }])).toBe(true);
    expect(hasRelayServer([{ urls: ['stun:s.example.com', 'turn:r.example.com'] }])).toBe(true);
  });

  it('handles empty and missing input', () => {
    expect(hasRelayServer([])).toBe(false);
    expect(hasRelayServer(undefined)).toBe(false);
  });
});

describe('openRelayFallbackServers', () => {
  it('uses the staticauth. hostname (the bare openrelay.metered.ca has no DNS records)', () => {
    const servers = openRelayFallbackServers();
    expect(servers.length).toBeGreaterThan(0);
    for (const s of servers) {
      expect(s.urls).toContain('staticauth.openrelay.metered.ca');
      expect(s.username).toBe('openrelayproject');
      expect(s.credential).toBe('openrelayproject');
    }
    expect(hasRelayServer(servers)).toBe(true);
  });

  it('includes a turns: (TLS) variant for networks that block plain UDP/TCP TURN', () => {
    expect(openRelayFallbackServers().some(s => s.urls.startsWith('turns:'))).toBe(true);
  });
});

describe('fetchIceServers', () => {
  it('falls back to OpenRelay TURN with relaySource=openrelay when fetch fails (network error)', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('Network error')));
    const { servers, relayAvailable, relaySource } = await fetchIceServers();
    expect(servers[0].urls).toBe('stun:stun.l.google.com:19302');
    expect(servers.some(s => String(s.urls).includes('staticauth.openrelay.metered.ca'))).toBe(true);
    expect(relayAvailable).toBe(true);
    expect(relaySource).toBe('openrelay');
    vi.unstubAllGlobals();
  });

  it('falls back to OpenRelay TURN when fetch returns non-ok status', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 503 }));
    const { servers, relayAvailable, relaySource } = await fetchIceServers();
    expect(servers.some(s => String(s.urls).includes('staticauth.openrelay.metered.ca'))).toBe(true);
    expect(relayAvailable).toBe(true);
    expect(relaySource).toBe('openrelay');
    vi.unstubAllGlobals();
  });

  it('appends worker servers with relaySource=worker when they include TURN', async () => {
    const extra = [{ urls: 'turn:example.com:3478', username: 'test', credential: 'pass' }];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: vi.fn().mockResolvedValue(extra),
    }));
    const { servers, relayAvailable, relaySource } = await fetchIceServers();
    expect(servers).toHaveLength(3);
    expect(servers[2].urls).toBe('turn:example.com:3478');
    expect(relayAvailable).toBe(true);
    expect(relaySource).toBe('worker');
    vi.unstubAllGlobals();
  });

  it('does not add OpenRelay when the worker responds ok with only STUN entries', async () => {
    const extra = [{ urls: 'stun:stun.example.com:80' }];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: vi.fn().mockResolvedValue(extra),
    }));
    const { servers, relayAvailable, relaySource } = await fetchIceServers();
    expect(servers.some(s => String(s.urls).includes('openrelay'))).toBe(false);
    expect(relayAvailable).toBe(false);
    expect(relaySource).toBe(null);
    vi.unstubAllGlobals();
  });
});

describe('connectTimeoutMs', () => {
  it('escalates 8s → 16s → 30s and holds at 30s', () => {
    expect(connectTimeoutMs(0)).toBe(8_000);
    expect(connectTimeoutMs(1)).toBe(16_000);
    expect(connectTimeoutMs(2)).toBe(30_000);
    expect(connectTimeoutMs(3)).toBe(30_000);
    expect(connectTimeoutMs(100)).toBe(30_000);
  });

  it('treats negative attempts as attempt 0', () => {
    expect(connectTimeoutMs(-1)).toBe(8_000);
  });

  it('respects a custom schedule', () => {
    expect(connectTimeoutMs(0, [100, 200])).toBe(100);
    expect(connectTimeoutMs(5, [100, 200])).toBe(200);
  });
});

describe('candidateType', () => {
  it('prefers the structured .type field', () => {
    expect(candidateType({ type: 'relay', candidate: 'ignored' })).toBe('relay');
  });

  it('falls back to parsing the SDP string', () => {
    expect(candidateType({ candidate: 'candidate:1 1 udp 2122260223 192.168.1.2 56789 typ host generation 0' })).toBe('host');
    expect(candidateType({ candidate: 'candidate:2 1 udp 1686052607 203.0.113.5 56789 typ srflx raddr 0.0.0.0' })).toBe('srflx');
  });

  it('returns null for end-of-candidates and missing input', () => {
    expect(candidateType(null)).toBeNull();
    expect(candidateType({ candidate: '' })).toBeNull();
  });
});

describe('probeTurnRelay', () => {
  it('resolves unavailable when the list has no TURN servers', async () => {
    await expect(probeTurnRelay(defaultIceServers())).resolves.toBe('unavailable');
  });

  it('resolves unavailable when RTCPeerConnection does not exist (Node)', async () => {
    await expect(probeTurnRelay([{ urls: 'turn:r.example.com:80' }])).resolves.toBe('unavailable');
  });

  it('resolves reachable when a relay candidate surfaces', async () => {
    // Under iceTransportPolicy 'relay' every candidate is a relay candidate,
    // so the probe treats the first one as proof of reachability.
    class FakePC {
      constructor() { this.onicecandidate = null; }
      createDataChannel() {}
      createOffer() { return Promise.resolve({}); }
      setLocalDescription() {
        queueMicrotask(() =>
          this.onicecandidate?.({ candidate: { candidate: 'candidate:1 1 udp 1 10.0.0.1 1 typ relay' } }));
        return Promise.resolve();
      }
      close() {}
    }
    vi.stubGlobal('RTCPeerConnection', FakePC);
    await expect(probeTurnRelay([{ urls: 'turn:r.example.com:80' }])).resolves.toBe('reachable');
    vi.unstubAllGlobals();
  });

  it('resolves unreachable when no candidate arrives before the timeout', async () => {
    class FakePC {
      constructor() { this.onicecandidate = null; }
      createDataChannel() {}
      createOffer() { return Promise.resolve({}); }
      setLocalDescription() { return Promise.resolve(); }
      close() {}
    }
    vi.stubGlobal('RTCPeerConnection', FakePC);
    vi.useFakeTimers();
    const p = probeTurnRelay([{ urls: 'turn:r.example.com:80' }], 5_000);
    await vi.advanceTimersByTimeAsync(5_000);
    await expect(p).resolves.toBe('unreachable');
    vi.useRealTimers();
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
