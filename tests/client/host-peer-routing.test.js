// tests/client/host-peer-routing.test.js — issue #1230: server.html's
// routeOutbound() closed over the live tokenConns/tokenSnapshotConns Maps and
// called `.send()` inline, so there was no importable target-resolution logic
// to test without a real PeerJS session. This suite exercises the pure
// resolution lifted out of it (gui/host-peer-routing.js) with plain
// `Map([[token, fakeConn]])` fixtures.

import { describe, it, expect } from 'vitest';
import { outboundTargets } from '../../gui/host-peer-routing.js';

/** A fake PeerJS-shaped DataConnection: `open` is the flag routeOutbound read. */
const conn = (open = true) => ({ open, sent: [] });
/** A fake raw-RTCDataChannel-shaped stub: `readyState` instead of `open`. */
const rtcConn = (readyState = 'open') => ({ readyState, sent: [] });

describe('outboundTargets — target resolution', () => {
  it('all: returns every open reliable connection, skipping closed ones', () => {
    const a = conn(true), b = conn(false), c = conn(true);
    const tokenConns = new Map([['a', a], ['b', b], ['c', c]]);
    const out = outboundTargets('all', 'reliable', { tokenConns, tokenSnapshotConns: new Map() });
    expect(out).toEqual([a, c]);
  });

  it('token:<id>: returns exactly that one connection when open', () => {
    const a = conn(true), b = conn(true);
    const tokenConns = new Map([['a', a], ['b', b]]);
    const out = outboundTargets('token:b', 'reliable', { tokenConns, tokenSnapshotConns: new Map() });
    expect(out).toEqual([b]);
  });

  it('token:<id>: returns nothing for an unknown or closed token', () => {
    const tokenConns = new Map([['a', conn(false)]]);
    expect(outboundTargets('token:a', 'reliable', { tokenConns, tokenSnapshotConns: new Map() })).toEqual([]);
    expect(outboundTargets('token:missing', 'reliable', { tokenConns, tokenSnapshotConns: new Map() })).toEqual([]);
  });

  it('except:<id>: returns every open connection except the named token', () => {
    const a = conn(true), b = conn(true), c = conn(true);
    const tokenConns = new Map([['a', a], ['b', b], ['c', c]]);
    const out = outboundTargets('except:b', 'reliable', { tokenConns, tokenSnapshotConns: new Map() });
    expect(out).toEqual([a, c]);
  });

  it('an unrecognised target shape resolves to no connections', () => {
    const tokenConns = new Map([['a', conn(true)]]);
    expect(outboundTargets('bogus', 'reliable', { tokenConns, tokenSnapshotConns: new Map() })).toEqual([]);
  });

  it('recognises open via readyState for a raw-channel-shaped connection', () => {
    const a = rtcConn('open'), b = rtcConn('connecting');
    const tokenConns = new Map([['a', a], ['b', b]]);
    const out = outboundTargets('all', 'reliable', { tokenConns, tokenSnapshotConns: new Map() });
    expect(out).toEqual([a]);
  });
});

describe('outboundTargets — snapshot delivery + reliable fallback', () => {
  it('prefers the snapshot connection when open', () => {
    const snap = conn(true), reliable = conn(true);
    const out = outboundTargets('token:a', 'snapshot', {
      tokenConns: new Map([['a', reliable]]),
      tokenSnapshotConns: new Map([['a', snap]]),
    });
    expect(out).toEqual([snap]);
  });

  it('falls back to the reliable connection when only the snapshot channel is missing', () => {
    const reliable = conn(true);
    const out = outboundTargets('token:a', 'snapshot', {
      tokenConns: new Map([['a', reliable]]),
      tokenSnapshotConns: new Map(),
    });
    expect(out).toEqual([reliable]);
  });

  it('falls back to the reliable connection when the snapshot channel exists but is closed', () => {
    const reliable = conn(true);
    const out = outboundTargets('token:a', 'snapshot', {
      tokenConns: new Map([['a', reliable]]),
      tokenSnapshotConns: new Map([['a', conn(false)]]),
    });
    expect(out).toEqual([reliable]);
  });

  it('never sends on both channels for the same token', () => {
    const snap = conn(true), reliable = conn(true);
    const out = outboundTargets('token:a', 'snapshot', {
      tokenConns: new Map([['a', reliable]]),
      tokenSnapshotConns: new Map([['a', snap]]),
    });
    expect(out).toHaveLength(1);
  });

  it('a reliable delivery never falls back to the snapshot channel', () => {
    // No 'a' entry in tokenConns at all — a reliable delivery must not borrow
    // the snapshot-only connection for it.
    const snap = conn(true);
    const out = outboundTargets('token:a', 'reliable', {
      tokenConns: new Map(),
      tokenSnapshotConns: new Map([['a', snap]]),
    });
    expect(out).toEqual([]);
  });

  it('all + snapshot: falls back per-token independently, not as an all-or-nothing switch', () => {
    // 'a' has a snapshot channel; 'b' only ever opened the reliable one.
    const snapA = conn(true), reliableA = conn(true), reliableB = conn(true);
    const out = outboundTargets('all', 'snapshot', {
      tokenConns: new Map([['a', reliableA], ['b', reliableB]]),
      tokenSnapshotConns: new Map([['a', snapA]]),
    });
    expect(out).toEqual([snapA, reliableB]);
  });

  it('except + snapshot: excludes the named token from both the primary pass and its fallback', () => {
    const reliableA = conn(true), reliableB = conn(true);
    const out = outboundTargets('except:a', 'snapshot', {
      tokenConns: new Map([['a', reliableA], ['b', reliableB]]),
      tokenSnapshotConns: new Map(), // neither token has a snapshot channel
    });
    expect(out).toEqual([reliableB]);
  });
});
