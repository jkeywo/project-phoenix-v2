// Rendezvous contract tests (issue #1111).
//
// These drive worker-rendezvous/src/registry.js — the whole service logic —
// through its public frame vocabulary: issue a code, resolve one, join, relay
// signalling, and get each of the three distinct lookup failures. No wrangler,
// no miniflare, no sockets; the Worker adapter around this module is a socket
// pump with nothing to decide.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  createRegistry,
  RENDEZVOUS_PROTOCOL,
  ROLE_HOST,
  ROLE_CLIENT,
} from '../../worker-rendezvous/src/registry.js';
import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  setJoinCodeData,
  projectGuidFor,
  versionGuid,
  composeJoinCode,
} from '../../gui/join-code.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(
  readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'),
);
setJoinCodeData(DATA);

const CLIENT_PROJECT = projectGuidFor(NAMESPACE_CLIENT, DATA);
const SERVER_PROJECT = projectGuidFor(NAMESPACE_SERVER, DATA);
const VERSION = versionGuid(DATA);

/** A registry plus the little bookkeeping every test repeats. */
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
    raw: (id, frame) => collect(reg.receive(id, frame)),
    disconnect: (id) => collect(reg.disconnect(id)),
    frames: (id) => inbox.get(id) || [],
    last: (id, type) => (inbox.get(id) || []).filter((f) => f.type === type).pop() || null,
  };
}

/** Open a host socket and return the code it was issued. */
function openHost(h, connId = 'host-1', namespace = NAMESPACE_CLIENT, extra = {}) {
  h.connect(connId, ROLE_HOST);
  h.send(connId, { type: 'host-open', namespace, ...extra });
  return h.last(connId, 'hosted').code;
}

/** The draws that mint one specific word, for scripting `randomInt`. */
const letters = (word) => [...word].map((c) => DATA.suffix.alphabet.indexOf(c));

/**
 * A release GUID that is not this build's. Registered versions must be
 * GUID-shaped — it is the one host-supplied value that becomes part of a
 * record key — so "another release" is another GUID, not a prose label.
 */
const OTHER_RELEASE = '11112222-3333-4444-5555-666677778888';

describe('code issue', () => {
  it('issues a five-letter code in the namespace the host asked for', () => {
    const h = harness();
    const code = openHost(h);
    expect(code.suffix).toHaveLength(DATA.suffix.length);
    expect(code.namespace).toBe(NAMESPACE_CLIENT);
    expect(code.project).toBe(CLIENT_PROJECT);
    expect(code.full).toBe(composeJoinCode(code));
  });

  it('never issues the same suffix twice in one namespace', () => {
    // Scripted so host-2's FIRST draw replays host-1's exact code and the
    // retry branch in mintSuffix genuinely executes. A draw sequence that
    // never repeats leaves that branch uncovered while the assertion below
    // still passes, which is what this test used to do.
    let i = 0;
    const script = [...letters('QUARK'), ...letters('QUARK'), ...letters('MOIST')];
    const h = harness({ randomInt: () => script[i++] });
    const first = openHost(h, 'host-1');
    const second = openHost(h, 'host-2');
    expect(first.suffix).toBe('QUARK');
    expect(second.suffix).toBe('MOIST');
    expect(i, 'the collision was never drawn, so the retry never ran').toBe(script.length);
    expect(h.reg.snapshot()).toHaveLength(2);
  });

  it('opens the server namespace so its codes exist, without making them joinable', () => {
    const h = harness();
    const code = openHost(h, 'fleet-host', NAMESPACE_SERVER);
    expect(code.namespace).toBe(NAMESPACE_SERVER);
    expect(code.project).toBe(SERVER_PROJECT);
    expect(h.reg.snapshot().map((r) => r.namespace)).toContain(NAMESPACE_SERVER);
  });

  it('refuses a second code on one host socket', () => {
    const h = harness();
    openHost(h);
    h.send('host-1', { type: 'host-open', namespace: NAMESPACE_CLIENT });
    expect(h.last('host-1', 'error')).toMatchObject({ reason: 'already-hosting' });
  });

  it('never issues a denied word', () => {
    // Force the first draw onto ADMIN; the mint must skip it.
    const a = DATA.suffix.alphabet;
    const script = [...[...'ADMIN'].map((c) => a.indexOf(c)), ...[...'QUARK'].map((c) => a.indexOf(c))];
    let i = 0;
    const h = harness({ randomInt: () => script[i++ % script.length] });
    expect(openHost(h).suffix).toBe('QUARK');
  });
});

describe('typed lookup', () => {
  it('resolves a code issued in the same namespace and release', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.full });
    expect(h.last('phone', 'resolved')).toMatchObject({
      namespace: NAMESPACE_CLIENT,
      admission: 'open',
    });
  });

  it('resolves the bare suffix a phone typed, without the full code', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix.toLowerCase() });
    expect(h.last('phone', 'resolved')).toBeTruthy();
  });

  it('answers unknown for a suffix nobody registered', () => {
    const h = harness();
    openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: 'ZZZZZ' });
    expect(h.last('phone', 'error')).toMatchObject({ request: 'resolve', reason: 'unknown' });
  });

  it('answers wrong-type for a suffix registered in the other namespace', () => {
    const h = harness();
    const code = openHost(h, 'fleet-host', NAMESPACE_SERVER);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'wrong-type' });
  });

  it('answers wrong-type when a full server code is presented to the crew join', () => {
    const h = harness();
    const code = openHost(h, 'fleet-host', NAMESPACE_SERVER);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.full });
    expect(h.last('phone', 'error')).toMatchObject({ request: 'join', reason: 'wrong-type' });
  });

  it('answers version-mismatch for the right namespace under another release', () => {
    const h = harness();
    const code = openHost(h, 'old-host', NAMESPACE_CLIENT, { version: OTHER_RELEASE });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', {
      type: 'resolve',
      code: composeJoinCode({ project: CLIENT_PROJECT, version: VERSION, suffix: code.suffix }),
    });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'version-mismatch' });
  });

  it('keeps unknown, wrong-type and version-mismatch as three different answers', () => {
    const h = harness();
    const server = openHost(h, 'fleet-host', NAMESPACE_SERVER);
    const old = openHost(h, 'old-host', NAMESPACE_CLIENT, { version: OTHER_RELEASE });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: 'ZZZZZ' });
    h.send('phone', { type: 'resolve', code: server.suffix });
    h.send('phone', {
      type: 'resolve',
      code: composeJoinCode({ project: CLIENT_PROJECT, version: VERSION, suffix: old.suffix }),
    });
    const reasons = h.frames('phone').filter((f) => f.type === 'error').map((f) => f.reason);
    expect(reasons).toEqual(['unknown', 'wrong-type', 'version-mismatch']);
  });

  it('refuses a denied suffix at lookup as well as at mint', () => {
    const h = harness();
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: 'ADMIN' });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'denied' });
  });

  it('refuses a frame from another protocol revision rather than guessing', () => {
    const h = harness();
    h.connect('phone', ROLE_CLIENT);
    h.raw('phone', { v: RENDEZVOUS_PROTOCOL + 1, type: 'resolve', code: 'ZZZZZ' });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'unsupported-protocol' });
  });
});

describe('admission state', () => {
  it('admits a join while admission is open and tells the host who arrived', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    expect(h.last('phone', 'joined')).toMatchObject({ peer: 'phone', admission: 'open' });
    expect(h.last('host-1', 'peer-joined')).toMatchObject({ peer: 'phone' });
    expect(h.reg.snapshot()[0].peers).toBe(1);
  });

  it('refuses a join once the host closes admission, without dropping the code', () => {
    const h = harness();
    const code = openHost(h);
    h.send('host-1', { type: 'host-admission', state: 'closed' });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'admission-closed' });
    expect(h.reg.snapshot()[0].admission).toBe('closed');
  });

  it('carries no build identity in either direction — that handshake is in-band', () => {
    // The host's authoritative check runs on the DataChannel
    // (delivery::check_join_stamp). A copy of either side's stamp relayed
    // through the service was read by nobody, so v1 does not carry one and
    // #1114/#1115 have no dead field to maintain.
    const h = harness();
    const code = openHost(h, 'host-1', NAMESPACE_CLIENT, { stamp: '1/phoenix-base/7' });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix });
    h.send('phone', { type: 'join', code: code.suffix, stamp: '1/phoenix-base/9' });
    expect(h.last('phone', 'resolved')).not.toHaveProperty('host_stamp');
    expect(h.last('phone', 'joined')).not.toHaveProperty('host_stamp');
    expect(h.last('host-1', 'peer-joined')).not.toHaveProperty('stamp');
  });

  it('refuses joiners past the authored per-record cap', () => {
    const cap = DATA.limits.max_peers_per_record;
    const h = harness();
    const code = openHost(h);
    for (let i = 0; i < cap; i += 1) {
      h.connect(`phone-${i}`, ROLE_CLIENT);
      h.send(`phone-${i}`, { type: 'join', code: code.suffix });
    }
    expect(h.reg.snapshot()[0].peers).toBe(cap);
    h.connect('one-too-many', ROLE_CLIENT);
    h.send('one-too-many', { type: 'join', code: code.suffix });
    expect(h.last('one-too-many', 'error')).toMatchObject({ reason: 'admission-closed' });
    expect(h.reg.snapshot()[0].peers).toBe(cap);
  });
});

describe('registry bounds', () => {
  it('cuts a socket off after the authored number of lookups', () => {
    const cap = DATA.limits.max_lookups_per_connection;
    const h = harness();
    openHost(h);
    h.connect('scanner', ROLE_CLIENT);
    for (let i = 0; i < cap; i += 1) h.send('scanner', { type: 'resolve', code: 'ZZZZZ' });
    expect(h.last('scanner', 'error')).toMatchObject({ reason: 'unknown' });

    // Past the cap the answer changes, and the frame tells the adapter to end
    // the socket rather than keep answering an enumeration.
    const [refusal] = h.reg.receive('scanner', { v: RENDEZVOUS_PROTOCOL, type: 'resolve', code: 'ZZZZZ' });
    expect(refusal.frame).toMatchObject({ type: 'error', reason: 'too-many-attempts' });
    expect(refusal.close).toBe(true);
  });

  it('refuses a code longer than a code can be, without parsing it', () => {
    const h = harness();
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: 'A'.repeat(DATA.limits.max_code_length + 1) });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'malformed' });
  });

  it('refuses a host release identifier that is not GUID-shaped', () => {
    // The version is the one host-supplied value that becomes part of a record
    // key, so it gets a shape rather than being concatenated as sent.
    const h = harness();
    h.connect('host-1', ROLE_HOST);
    h.send('host-1', { type: 'host-open', namespace: NAMESPACE_CLIENT, version: 'x'.repeat(4096) });
    expect(h.last('host-1', 'error')).toMatchObject({ request: 'host-open', reason: 'malformed' });
    expect(h.reg.snapshot()).toHaveLength(0);
  });

  it('expires a record whose TTL has run out and tells its joiners', () => {
    // The record normally dies with the host socket; this is the sweep for the
    // close that never arrived, so a code cannot be held by a host that is not
    // there any more.
    let clock = 1_000_000;
    const h = harness({ now: () => clock });
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });

    clock += DATA.limits.record_ttl_seconds * 1000 + 1;
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'closed')).toMatchObject({ reason: 'host-gone' });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'unknown' });
    expect(h.reg.snapshot()).toHaveLength(0);
  });

  it('does not expire a record whose host keeps sending frames, however long it has run', () => {
    // The TTL used to be measured from createdAt, so a host that had simply
    // been live for a long time was as expirable as an orphan. It is measured
    // from lastSeen now, refreshed by every inbound host frame — so a host
    // that keeps talking survives arbitrarily far past one TTL window's worth
    // of wall-clock time since it was minted.
    let clock = 1_000_000;
    const h = harness({ now: () => clock });
    const code = openHost(h);
    const ttl = DATA.limits.record_ttl_seconds * 1000;

    // Two keepalives, each well inside the TTL of the one before it, but
    // whose COMBINED span is more than two full TTL windows since createdAt.
    clock += ttl - 1000;
    h.send('host-1', { type: 'host-admission', state: 'open' });
    clock += ttl - 1000;
    h.send('host-1', { type: 'host-admission', state: 'open' });

    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'resolved')).toMatchObject({ admission: 'open' });
    expect(h.reg.snapshot()).toHaveLength(1);
  });

  it('expires an idle host past its TTL and tells the host its own code is gone', () => {
    // dropRecord used to notify only the record's joiners, so an idle host's
    // own viewscreen kept showing a code the service had already forgotten.
    let clock = 1_000_000;
    const h = harness({ now: () => clock });
    openHost(h);

    clock += DATA.limits.record_ttl_seconds * 1000 + 1;
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: 'ZZZZZ' });

    expect(h.reg.snapshot()).toHaveLength(0);
    // 'unreachable' is what createRendezvousHost's own socket.onerror/onclose
    // report — the frame its host-side handler already treats as "the record
    // is gone" — so the host's stale code clears the same way a real drop
    // would clear it, rather than needing a bespoke frame type.
    expect(h.last('host-1', 'error')).toMatchObject({ reason: 'unreachable' });
  });

  it('lets a host socket ask nothing of the client namespace', () => {
    // clientJoin has always had this guard; resolve did not, so a /v1/host
    // socket could read the crew namespace it is not served for.
    const h = harness();
    const code = openHost(h);
    h.connect('other-host', ROLE_HOST);
    h.send('other-host', { type: 'resolve', code: code.suffix });
    expect(h.last('other-host', 'error')).toMatchObject({
      request: 'resolve',
      reason: 'forbidden-role',
    });
    h.send('other-host', { type: 'join', code: code.suffix });
    expect(h.last('other-host', 'error')).toMatchObject({ reason: 'forbidden-role' });
  });
});

describe('signalling relay', () => {
  it('carries a frame from the joiner to its host and back', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });

    h.send('phone', { type: 'signal', payload: { sdp: 'offer' } });
    expect(h.last('host-1', 'signal')).toMatchObject({ from: 'phone', payload: { sdp: 'offer' } });

    h.send('host-1', { type: 'signal', to: 'phone', payload: { sdp: 'answer' } });
    expect(h.last('phone', 'signal')).toMatchObject({ from: 'host-1', payload: { sdp: 'answer' } });
  });

  it('will not relay to a peer that never joined this host', () => {
    const h = harness();
    openHost(h);
    h.connect('stranger', ROLE_CLIENT);
    h.send('host-1', { type: 'signal', to: 'stranger', payload: {} });
    expect(h.last('host-1', 'error')).toMatchObject({ request: 'signal', reason: 'no-peer' });
  });

  it('will not relay from a socket that has not joined anything', () => {
    const h = harness();
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'signal', payload: {} });
    expect(h.last('phone', 'error')).toMatchObject({ request: 'signal' });
  });

  it('keeps two joiners of one host apart', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('a', ROLE_CLIENT);
    h.connect('b', ROLE_CLIENT);
    h.send('a', { type: 'join', code: code.suffix });
    h.send('b', { type: 'join', code: code.suffix });
    h.send('host-1', { type: 'signal', to: 'a', payload: { for: 'a' } });
    expect(h.last('a', 'signal').payload).toEqual({ for: 'a' });
    expect(h.frames('b').filter((f) => f.type === 'signal')).toHaveLength(0);
  });
});

describe('code lifecycle', () => {
  it('tells joiners the host is gone and frees the code when the host socket drops', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    h.disconnect('host-1');
    expect(h.last('phone', 'closed')).toMatchObject({ reason: 'host-gone' });
    expect(h.reg.snapshot()).toHaveLength(0);
    // and the freed suffix is unknown again rather than half-alive
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'unknown' });
  });

  it('reports a departing joiner to its host and keeps the code alive', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    h.disconnect('phone');
    expect(h.last('host-1', 'peer-left')).toMatchObject({ peer: 'phone' });
    expect(h.reg.snapshot()[0].peers).toBe(0);
  });

  it('keeps a code private — the diagnostics snapshot names no code, peer or stamp', () => {
    const h = harness();
    const code = openHost(h, 'host-1', NAMESPACE_CLIENT, { stamp: '1/phoenix-base/1' });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    const [row] = h.reg.snapshot();
    expect(row).toEqual({
      namespace: NAMESPACE_CLIENT,
      version: VERSION,
      admission: 'open',
      peers: 1,
    });
    // The suffix IS the private client code. A view that carries it is a
    // diagnostics endpoint one route away from handing out every live session.
    expect(JSON.stringify(h.reg.snapshot())).not.toContain(code.suffix);
  });
});
