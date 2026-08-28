// The secure-WebSocket game relay (issue #1113).
//
// Two layers, tested separately because they prove different things:
//
//   1. worker-rendezvous/src/relay.js on its own — the bounded mailbox. This is
//      where the reliable/snapshot asymmetry lives, and it is the ONLY place a
//      test can reach the bound without pretending a socket is saturated.
//   2. the relay verbs through worker-rendezvous/src/registry.js — who may
//      attach, where a frame goes, and what happens to a relayed peer when the
//      record it is riding on dies.
//
// Both run headless with no wrangler, no miniflare and no sockets, exactly as
// the #1111 contract tests do. What they DO NOT prove is that a real Cloudflare
// WebSocket carries these frames — that is the deploy check
// (scripts/check-rendezvous.mjs) and the human acceptance kit
// (docs/acceptance/1113-networks.md), and neither can run in CI.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  createRelayHub,
  relayLimits,
  isRelayClass,
  payloadBytes,
  RELAY_RELIABLE,
  RELAY_SNAPSHOT,
} from '../../worker-rendezvous/src/relay.js';
import {
  createRegistry,
  RENDEZVOUS_PROTOCOL,
  ROLE_HOST,
  ROLE_CLIENT,
} from '../../worker-rendezvous/src/registry.js';
import { NAMESPACE_CLIENT, setJoinCodeData } from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(
  readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'),
);
setJoinCodeData(DATA);

// ── The mailbox itself ──────────────────────────────────────────────────────

describe('relay bounds', () => {
  it('reads its bounds from the authored limits table', () => {
    // The numbers are a designer's, not this module's — AGENTS.md rule 11. The
    // assertion is that the shipped table is what a hub is built from, so a
    // tuning edit in assets/join/join-codes.toml actually reaches the service.
    const bounds = relayLimits(DATA.limits);
    expect(bounds.maxPeers).toBe(DATA.limits.max_relay_peers_per_record);
    expect(bounds.maxFrameBytes).toBe(DATA.limits.max_relay_frame_bytes);
    expect(bounds.maxReliable).toBe(DATA.limits.max_relay_queue_reliable);
    expect(bounds.maxSnapshot).toBe(DATA.limits.max_relay_queue_snapshot);
  });

  it('still loads against a table that predates the relay', () => {
    // The #1111 limits with none of #1113's, which is what a worker deployed
    // from an older bundle would carry.
    const bounds = relayLimits({ max_peers_per_record: 32 });
    expect(bounds.maxPeers).toBeGreaterThan(0);
    expect(bounds.maxSnapshot).toBeGreaterThan(0);
  });

  it('names exactly the two delivery classes the game has', () => {
    expect(isRelayClass(RELAY_RELIABLE)).toBe(true);
    expect(isRelayClass(RELAY_SNAPSHOT)).toBe(true);
    expect(isRelayClass('unordered')).toBe(false);
    expect(isRelayClass(undefined)).toBe(false);
  });

  it('measures a payload in bytes rather than UTF-16 units', () => {
    // A display name or a comms line is not ASCII, and a bound written against
    // String.length would silently be a larger bound than the authored one.
    expect(payloadBytes('abc')).toBe(3);
    expect(payloadBytes('€')).toBe(3);
    expect(payloadBytes(null)).toBe(Infinity);
  });
});

describe('the bounded mailbox', () => {
  const hub = (over = {}) =>
    createRelayHub({
      limits: {
        max_relay_peers_per_record: 2,
        max_relay_frame_bytes: 32,
        max_relay_queue_reliable: 3,
        max_relay_queue_snapshot: 2,
        ...over,
      },
    });

  it('drops the OLDEST snapshot frames once the snapshot queue is full', () => {
    const h = hub();
    h.attach('peer-1', 'record');
    const result = [1, 2, 3, 4].map((n) =>
      h.enqueue('peer-1', 'host', RELAY_SNAPSHOT, { n }, `${n}`),
    );
    // Two shed, and the two SURVIVORS are the newest — the whole point of the
    // lossy class is that a late snapshot is worthless.
    expect(result.map((r) => r.dropped)).toEqual([0, 0, 1, 1]);
    // The RUNNING TOTAL is the number a sender is told, because a per-enqueue
    // delta is 1 essentially always once a queue is sitting at its bound — a
    // readout folding deltas said "1" however many hundreds were lost.
    expect(result.map((r) => r.totalDropped)).toEqual([0, 0, 1, 2]);
    expect(h.drain('peer-1').map((f) => f.n)).toEqual([3, 4]);
    expect(h.stats('peer-1').dropped).toBe(2);
  });

  it('never drops a reliable frame, and marks the session dead instead', () => {
    const h = hub();
    h.attach('peer-1', 'record');
    for (const n of [1, 2, 3]) h.enqueue('peer-1', 'host', RELAY_RELIABLE, { n }, `${n}`);
    expect(h.hasOverflowed('peer-1')).toBe(false);
    h.enqueue('peer-1', 'host', RELAY_RELIABLE, { n: 4 }, '4');
    expect(h.hasOverflowed('peer-1')).toBe(true);
    expect(h.overflowedSources('peer-1')).toEqual(['host']);
    // Everything that was queued is still there. A command the service accepted
    // and then silently discarded would be worse than a closed session, because
    // the game would carry on believing it had been sent.
    expect(h.drain('peer-1').map((f) => f.n)).toEqual([1, 2, 3, 4]);
  });

  it('drains reliable frames ahead of snapshot ones', () => {
    const h = hub();
    h.attach('peer-1', 'record');
    h.enqueue('peer-1', 'host', RELAY_SNAPSHOT, { s: 1 }, 's');
    h.enqueue('peer-1', 'host', RELAY_RELIABLE, { r: 1 }, 'r');
    const [first, second] = h.drain('peer-1');
    expect(first).toEqual({ r: 1 });
    expect(second).toEqual({ s: 1 });
  });

  it('bounds each SENDER separately, so one phone cannot evict another', () => {
    // Every relaying joiner sends to the same host, so a mailbox keyed by the
    // DESTINATION alone made both bounds record-wide by accident: one phone's
    // reliable burst overflowed the box every other phone was queued in, and
    // one phone's snapshots displaced another's. The bound has to bite the
    // pair that caused it.
    const h = hub();
    h.attach('host', 'record', { counted: false });
    h.attach('a', 'record');
    h.attach('b', 'record');

    // A floods the host past the reliable bound; B queues one frame.
    for (let i = 0; i <= 3; i += 1) h.enqueue('host', 'a', RELAY_RELIABLE, { a: i }, `${i}`);
    h.enqueue('host', 'b', RELAY_RELIABLE, { b: 1 }, 'b');
    expect(h.overflowedSources('host')).toEqual(['a']);

    // B's snapshots are its own too: A's burst cannot displace them.
    h.enqueue('host', 'b', RELAY_SNAPSHOT, { b: 's1' }, 's');
    for (let i = 0; i < 5; i += 1) h.enqueue('host', 'a', RELAY_SNAPSHOT, { a: i }, 's');
    expect(h.drain('host')).toContainEqual({ b: 's1' });

    // …and ending A's session leaves B relaying, with a mailbox of its own.
    h.detach('a');
    expect(h.overflowedSources('host')).toEqual([]);
    h.enqueue('host', 'b', RELAY_RELIABLE, { b: 2 }, 'b');
    expect(h.drain('host')).toEqual([{ b: 2 }]);
  });

  it('refuses a frame larger than the authored ceiling', () => {
    const h = hub();
    h.attach('peer-1', 'record');
    expect(h.enqueue('peer-1', 'host', RELAY_RELIABLE, {}, 'x'.repeat(33))).toEqual({
      ok: false,
      reason: 'relay-too-large',
    });
  });

  it('refuses a class it cannot honour rather than guessing one', () => {
    const h = hub();
    h.attach('peer-1', 'record');
    expect(h.enqueue('peer-1', 'host', 'best-effort', {}, 'x').reason).toBe('malformed');
  });

  it('bounds how many joiners one record may relay', () => {
    const h = hub();
    expect(h.attach('a', 'record').ok).toBe(true);
    expect(h.attach('b', 'record').ok).toBe(true);
    expect(h.attach('c', 'record')).toEqual({ ok: false, reason: 'relay-full' });
    // Another record's relay is unaffected — the bound is per record.
    expect(h.attach('c', 'other').ok).toBe(true);
  });

  it('lets a record host hold a mailbox without spending a crew slot', () => {
    const h = hub();
    h.attach('host', 'record', { counted: false });
    expect(h.attach('a', 'record').ok).toBe(true);
    expect(h.attach('b', 'record').ok).toBe(true);
    expect(h.peersFor('record')).toEqual(['a', 'b']);
    expect(h.isAttached('host')).toBe(true);
  });

  it('discards a detached peer’s queue rather than leaking it', () => {
    const h = hub();
    h.attach('host', 'record', { counted: false });
    h.attach('peer-1', 'record');
    h.enqueue('peer-1', 'host', RELAY_RELIABLE, { n: 1 }, '1');
    // The other end of the same pair, which a detach must also clear: leaving
    // it behind would leak a queue nothing will ever drain.
    h.enqueue('host', 'peer-1', RELAY_RELIABLE, { n: 2 }, '2');
    h.detach('peer-1');
    expect(h.drain('peer-1')).toEqual([]);
    expect(h.drain('host')).toEqual([]);
    expect(h.stats('peer-1')).toBeNull();
    expect(h.enqueue('peer-1', 'host', RELAY_RELIABLE, {}, 'x').reason).toBe('not-relaying');
  });
});

// ── The relay verbs, through the real registry ──────────────────────────────

/** A registry plus the bookkeeping every case below repeats. */
function harness(opts = {}) {
  const reg = createRegistry({ data: DATA, ...opts });
  const inbox = new Map();
  const collect = (frames) => {
    for (const { to, frame } of frames) {
      if (!inbox.has(to)) inbox.set(to, []);
      inbox.get(to).push(frame);
    }
    return frames;
  };
  return {
    reg,
    connect: (id, role) => collect(reg.connect(id, role)),
    send: (id, frame) => collect(reg.receive(id, { v: RENDEZVOUS_PROTOCOL, ...frame })),
    disconnect: (id) => collect(reg.disconnect(id)),
    frames: (id) => inbox.get(id) || [],
    last: (id, type) => (inbox.get(id) || []).filter((f) => f.type === type).pop() || null,
  };
}

/** A host holding a code, with `peers` joined clients attached to it. */
function session(h, peers = ['peer-1']) {
  h.connect('host-1', ROLE_HOST);
  h.send('host-1', { type: 'host-open', namespace: NAMESPACE_CLIENT });
  const code = h.last('host-1', 'hosted').code;
  for (const id of peers) {
    h.connect(id, ROLE_CLIENT);
    h.send(id, { type: 'join', code: code.full });
  }
  return code;
}

describe('attaching to the relay', () => {
  it('answers a joined client with relay-ready and tells the host', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    const ready = h.last('peer-1', 'relay-ready');
    expect(ready.peer).toBe('peer-1');
    // The bounds travel to the client so its own transport can refuse an
    // oversized frame locally instead of discovering the ceiling by being cut
    // off at it.
    expect(ready.limits.max_frame_bytes).toBe(DATA.limits.max_relay_frame_bytes);
    expect(h.last('host-1', 'relay-peer')).toMatchObject({ peer: 'peer-1' });
  });

  it('refuses a client that has not joined a record', () => {
    const h = harness();
    session(h, []);
    h.connect('stranger', ROLE_CLIENT);
    h.send('stranger', { type: 'relay-open' });
    expect(h.last('stranger', 'error')).toMatchObject({
      request: 'relay-open',
      reason: 'not-joined',
    });
  });

  it('refuses a host socket asking to relay as if it were crew', () => {
    const h = harness();
    session(h, []);
    h.send('host-1', { type: 'relay-open' });
    expect(h.last('host-1', 'error')).toMatchObject({ reason: 'forbidden-role' });
  });

  it('refuses a record that has closed its crew list', () => {
    const h = harness();
    session(h);
    h.send('host-1', { type: 'host-admission', state: 'closed' });
    h.send('peer-1', { type: 'relay-open' });
    expect(h.last('peer-1', 'error')).toMatchObject({ reason: 'admission-closed' });
  });

  it('counts relayed crew separately in the diagnostics snapshot', () => {
    const h = harness();
    session(h, ['peer-1', 'peer-2']);
    h.send('peer-1', { type: 'relay-open' });
    // Two joiners, one of whom could not build a direct link. That difference
    // is exactly what an operator wants to see, and it carries no id or suffix
    // — same rule as the rest of `snapshot()`.
    expect(h.reg.snapshot()[0]).toMatchObject({ peers: 2, relayPeers: 1 });
  });
});

describe('carrying game frames', () => {
  it('carries an opaque payload from a relayed client to its host', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    // Deliberately the real thing: the same ClientMessage JSON a DataChannel
    // would have carried, byte for byte. The service must move it without
    // knowing what it is.
    const payload = JSON.stringify({ type: 'Identify', data: { token: 'abc', name: 'Ada' } });
    h.send('peer-1', { type: 'relay', class: 'reliable', payload });
    expect(h.last('host-1', 'relay')).toEqual({
      v: RENDEZVOUS_PROTOCOL,
      type: 'relay',
      from: 'peer-1',
      class: 'reliable',
      payload,
    });
  });

  it('carries a host frame to the relayed peer it names', () => {
    const h = harness();
    session(h, ['peer-1', 'peer-2']);
    h.send('peer-1', { type: 'relay-open' });
    h.send('peer-2', { type: 'relay-open' });
    h.send('host-1', { type: 'relay', to: 'peer-2', class: 'snapshot', payload: '{"type":"SimState"}' });
    expect(h.last('peer-2', 'relay')).toMatchObject({ class: 'snapshot' });
    expect(h.last('peer-1', 'relay')).toBeNull();
  });

  it('refuses a host frame aimed at a peer that is not relaying', () => {
    // A peer with a perfectly good DataChannel must not be reachable this way:
    // two live paths to one crew member is a duplicate-delivery bug waiting to
    // be found in the field.
    const h = harness();
    session(h, ['peer-1', 'peer-2']);
    h.send('peer-1', { type: 'relay-open' });
    h.send('host-1', { type: 'relay', to: 'peer-2', class: 'reliable', payload: '{}' });
    expect(h.last('host-1', 'error')).toMatchObject({ request: 'relay', reason: 'no-peer' });
  });

  it('refuses a game frame from a client that never attached', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay', class: 'reliable', payload: '{}' });
    expect(h.last('peer-1', 'error')).toMatchObject({ reason: 'not-relaying' });
  });

  it('refuses an oversized frame instead of storing it', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    const payload = 'x'.repeat(DATA.limits.max_relay_frame_bytes + 1);
    h.send('peer-1', { type: 'relay', class: 'reliable', payload });
    expect(h.last('peer-1', 'error')).toMatchObject({ reason: 'relay-too-large' });
    expect(h.last('host-1', 'relay')).toBeNull();
  });

  it('delivers every frame while the target socket is writable', () => {
    // The ordinary case, stated so the shedding cases below are read as the
    // exception they are: a Durable Object drains a mailbox on the same
    // synchronous turn it fills it, so nothing queues and nothing sheds.
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    const n = DATA.limits.max_relay_queue_snapshot + 3;
    for (let i = 0; i < n; i += 1) {
      h.send('host-1', { type: 'relay', to: 'peer-1', class: 'snapshot', payload: `${i}` });
    }
    expect(h.frames('peer-1').filter((f) => f.type === 'relay')).toHaveLength(n);
    expect(h.last('host-1', 'relay-degraded')).toBeNull();
  });

  it('sheds snapshot frames — and tells the sender — while the target cannot take bytes', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    h.reg.setWritable('peer-1', false);

    const depth = DATA.limits.max_relay_queue_snapshot;
    for (let i = 0; i < depth + 3; i += 1) {
      h.send('host-1', { type: 'relay', to: 'peer-1', class: 'snapshot', payload: `${i}` });
    }
    // The sender is the one told, because the sender is the one that can slow
    // down and the one whose operator is looking at a diagnostics readout.
    const degraded = h.frames('host-1').filter((f) => f.type === 'relay-degraded');
    expect(degraded).toHaveLength(3);
    // A RUNNING TOTAL, not this enqueue's delta. The delta is 1 every time once
    // the queue is at its bound, so a readout built on it reads "1" for the
    // whole mission however many hundreds of frames are actually being lost.
    expect(degraded.map((f) => f.dropped)).toEqual([1, 2, 3]);
    expect(degraded[0]).toMatchObject({ peer: 'peer-1', class: 'snapshot' });

    // What survives is the NEWEST window, which is the whole point of the
    // lossy class: the stale snapshots are the ones worth losing.
    const delivered = h.reg.setWritable('peer-1', true).map((f) => f.frame.payload);
    expect(delivered).toEqual(
      Array.from({ length: depth }, (_, i) => `${i + 3}`),
    );
  });

  it('never sheds a reliable frame, and closes the session when it cannot hold one more', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    h.reg.setWritable('peer-1', false);

    const depth = DATA.limits.max_relay_queue_reliable;
    for (let i = 0; i < depth; i += 1) {
      h.send('host-1', { type: 'relay', to: 'peer-1', class: 'reliable', payload: `${i}` });
    }
    // Nothing was shed and nothing was reported as shed — a silently dropped
    // command would break the guarantee the game is written against. Every one
    // of them is still held, and comes out in order once the peer can take it.
    expect(h.frames('host-1').filter((f) => f.type === 'relay-degraded')).toHaveLength(0);
    const released = h.reg.setWritable('peer-1', true).map((f) => f.frame.payload);
    expect(released).toEqual(Array.from({ length: depth }, (_, i) => `${i}`));

    // One past the bound is where the promise breaks. The session ends, with a
    // reason both ends can render, rather than carrying on as a reliable
    // channel that is quietly not reliable.
    h.reg.setWritable('peer-1', false);
    for (let i = 0; i <= depth; i += 1) {
      h.send('host-1', { type: 'relay', to: 'peer-1', class: 'reliable', payload: `over-${i}` });
    }
    expect(h.frames('host-1').filter((f) => f.type === 'relay-degraded')).toHaveLength(0);
    expect(h.last('peer-1', 'relay-closed')).toMatchObject({ reason: 'relay-overflow' });
    expect(h.last('host-1', 'relay-peer-left')).toMatchObject({ reason: 'relay-overflow' });
    expect(h.reg.snapshot()[0].relayPeers).toBe(0);
  });
});

  it('ends only the offending phone’s session when the host’s mailbox overflows', () => {
    // Every relaying joiner sends to the same host. Scoped to the host's
    // CONNECTION, one phone's reliable burst detached the host's mailbox and
    // left every other crew member answering `not-relaying` for the rest of
    // the mission — one guest's bad radio ending everybody else's game.
    const h = harness();
    session(h, ['noisy', 'quiet']);
    h.send('noisy', { type: 'relay-open' });
    h.send('quiet', { type: 'relay-open' });
    h.reg.setWritable('host-1', false);

    const depth = DATA.limits.max_relay_queue_reliable;
    for (let i = 0; i <= depth; i += 1) {
      h.send('noisy', { type: 'relay', class: 'reliable', payload: `${i}` });
    }
    // The phone that overflowed is the session that ends, and it is told why.
    expect(h.last('noisy', 'relay-closed')).toMatchObject({ reason: 'relay-overflow' });
    expect(h.last('host-1', 'relay-peer-left')).toMatchObject({
      peer: 'noisy',
      reason: 'relay-overflow',
    });
    // The quiet one is untouched and still carried.
    expect(h.last('quiet', 'relay-closed')).toBeNull();
    expect(h.reg.snapshot()[0].relayPeers).toBe(1);
    h.reg.setWritable('host-1', true);
    h.send('quiet', { type: 'relay', class: 'reliable', payload: 'still here' });
    expect(h.last('quiet', 'error')).toBeNull();
    expect(h.last('host-1', 'relay')).toMatchObject({ from: 'quiet', payload: 'still here' });
  });
});

describe('the relay’s lifetime', () => {
  it('lets a host detach one crew member, and tells that phone', () => {
    // The host's only eviction mechanism is closing the connection — the
    // reserved-token refusal and the duplicate-token dance in server.html. For
    // a WebRTC peer that severs real DataChannels the phone observes; for a
    // relayed peer it closed nothing but local JavaScript, so the evicted
    // device sat on a status line reading "connected", sending commands the
    // host dropped on the floor.
    const h = harness();
    session(h, ['peer-1', 'peer-2']);
    h.send('peer-1', { type: 'relay-open' });
    h.send('peer-2', { type: 'relay-open' });

    h.send('host-1', { type: 'relay-close', to: 'peer-1' });
    expect(h.last('peer-1', 'relay-closed')).toMatchObject({ reason: 'host-closed' });
    expect(h.last('host-1', 'relay-peer-left')).toMatchObject({ peer: 'peer-1' });
    expect(h.reg.snapshot()[0].relayPeers).toBe(1);
    // The other crew member is untouched.
    expect(h.last('peer-2', 'relay-closed')).toBeNull();
  });

  it('refuses a host detaching a peer that is not on its own relay', () => {
    const h = harness();
    session(h, ['peer-1']);
    h.send('peer-1', { type: 'relay-open' });
    h.send('host-1', { type: 'relay-close', to: 'somebody-else' });
    expect(h.last('host-1', 'error')).toMatchObject({
      request: 'relay-close',
      reason: 'no-peer',
    });
    expect(h.reg.snapshot()[0].relayPeers).toBe(1);
  });

  it('refuses a host stepping off its OWN relay, which would end everyone’s', () => {
    // `relay-close` with no `to` detaches the sender. From a host that would
    // silently end the relay for every crew member on the record, with no
    // notification to any of them — a footgun waiting for the next issue that
    // reaches for this verb.
    const h = harness();
    session(h, ['peer-1']);
    h.send('peer-1', { type: 'relay-open' });
    h.send('host-1', { type: 'relay-close' });
    expect(h.last('host-1', 'error')).toMatchObject({
      request: 'relay-close',
      reason: 'forbidden-role',
    });
    h.send('peer-1', { type: 'relay', class: 'reliable', payload: 'still carried' });
    expect(h.last('host-1', 'relay')).toMatchObject({ payload: 'still carried' });
  });

  it('tells a relayed peer the relay is gone when the record dies', () => {
    // A DataChannel outlives the record that introduced it — that is #1112's
    // whole two-planes rule. A RELAYED link does not: the record IS the link,
    // so the peer must be told, or it sits on a socket that will never carry
    // another game frame.
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    h.disconnect('host-1');
    expect(h.last('peer-1', 'relay-closed')).toMatchObject({ reason: 'host-gone' });
    expect(h.last('peer-1', 'closed')).toMatchObject({ reason: 'host-gone' });
  });

  it('tells the host when a relayed peer’s socket goes', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    h.disconnect('peer-1');
    expect(h.last('host-1', 'relay-peer-left')).toMatchObject({ peer: 'peer-1' });
  });

  it('lets a peer step off the relay without leaving the record', () => {
    const h = harness();
    session(h);
    h.send('peer-1', { type: 'relay-open' });
    h.send('peer-1', { type: 'relay-close' });
    expect(h.last('host-1', 'relay-peer-left')).toMatchObject({ peer: 'peer-1' });
    expect(h.reg.snapshot()[0]).toMatchObject({ peers: 1, relayPeers: 0 });
    // Still joined: signalling keeps working, which is what a client that has
    // just got a direct link needs.
    h.send('peer-1', { type: 'signal', payload: { candidate: {} } });
    expect(h.last('host-1', 'signal')).toBeTruthy();
  });

  it('frees a relay slot when a relayed peer leaves', () => {
    const h = harness();
    const code = session(h, []);
    const ids = [];
    for (let i = 0; i < DATA.limits.max_relay_peers_per_record; i += 1) {
      const id = `peer-${i}`;
      ids.push(id);
      h.connect(id, ROLE_CLIENT);
      h.send(id, { type: 'join', code: code.full });
      h.send(id, { type: 'relay-open' });
    }
    h.connect('one-too-many', ROLE_CLIENT);
    h.send('one-too-many', { type: 'join', code: code.full });
    h.send('one-too-many', { type: 'relay-open' });
    expect(h.last('one-too-many', 'error')).toMatchObject({ reason: 'relay-full' });

    h.disconnect(ids[0]);
    h.send('one-too-many', { type: 'relay-open' });
    expect(h.last('one-too-many', 'relay-ready')).toBeTruthy();
  });
});

describe('what a host says it can answer on', () => {
  it('tells a joiner both rungs by default, so an older host still works', () => {
    const h = harness();
    session(h);
    expect(h.last('peer-1', 'joined').transports).toEqual(['webrtc', 'ws-relay']);
  });

  it('relays a relay-only claim, which is what a native host is', () => {
    // A native host is a Rust process with a WebSocket and no WebRTC at all.
    // Without this its crew would spend the whole 8/16/30 s ladder discovering
    // that, four times over, before falling back to the path that was always
    // the only one.
    const h = harness();
    h.connect('host-1', ROLE_HOST);
    h.send('host-1', { type: 'host-open', namespace: NAMESPACE_CLIENT, transports: ['ws-relay'] });
    const code = h.last('host-1', 'hosted').code;
    h.connect('peer-1', ROLE_CLIENT);
    h.send('peer-1', { type: 'join', code: code.full });
    expect(h.last('peer-1', 'joined').transports).toEqual(['ws-relay']);
  });

  it('drops a claim it does not understand rather than relaying it', () => {
    // The field reaches a joiner that branches on it, so an unrecognised name
    // must not become a rung nobody implements.
    const h = harness();
    h.connect('host-1', ROLE_HOST);
    h.send('host-1', { type: 'host-open', namespace: NAMESPACE_CLIENT, transports: ['quic', 'ws-relay'] });
    const code = h.last('host-1', 'hosted').code;
    h.connect('peer-1', ROLE_CLIENT);
    h.send('peer-1', { type: 'join', code: code.full });
    expect(h.last('peer-1', 'joined').transports).toEqual(['ws-relay']);
  });

  it('reads a claim of nothing at all as a claim of everything', () => {
    // Better a host that is dialled on a rung it cannot answer — which fails
    // visibly and falls back — than one nobody tries at all.
    const h = harness();
    h.connect('host-1', ROLE_HOST);
    h.send('host-1', { type: 'host-open', namespace: NAMESPACE_CLIENT, transports: [] });
    const code = h.last('host-1', 'hosted').code;
    h.connect('peer-1', ROLE_CLIENT);
    h.send('peer-1', { type: 'join', code: code.full });
    expect(h.last('peer-1', 'joined').transports).toEqual(['webrtc', 'ws-relay']);
  });
});
