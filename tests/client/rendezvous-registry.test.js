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

  it('issues the privileged fleet code in the server namespace', () => {
    const h = harness();
    const code = openHost(h, 'fleet-host', NAMESPACE_SERVER);
    expect(code.namespace).toBe(NAMESPACE_SERVER);
    expect(code.project).toBe(SERVER_PROJECT);
    expect(h.reg.snapshot().map((r) => r.namespace)).toContain(NAMESPACE_SERVER);
  });

  it('lets one page hold a crew code and a fleet code at once, on two sockets', () => {
    // What a fleet lead actually is: the same browser tab registered twice,
    // once per namespace. The two records are independent keys, so neither
    // sees the other's admission state, peers or lifetime.
    const h = harness();
    const crew = openHost(h, 'crew-socket', NAMESPACE_CLIENT);
    const fleet = openHost(h, 'fleet-socket', NAMESPACE_SERVER);
    expect(h.reg.snapshot()).toHaveLength(2);
    h.send('fleet-socket', { type: 'host-admission', state: 'closed' });
    const byNamespace = Object.fromEntries(
      h.reg.snapshot().map((r) => [r.namespace, r.admission]),
    );
    expect(byNamespace).toEqual({ client: 'open', server: 'closed' });
    // And the two codes are different words in different namespaces.
    expect(crew.project).not.toBe(fleet.project);
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

  it('leaves an already-admitted joiner alone when admission closes', () => {
    // The lever the fleet operator pulls before mission start (issue #1114).
    // Closing must answer FUTURE joiners and nothing else: a ship already in
    // the fleet keeps its presence, gets no frame at all, and is admitted again
    // the moment the operator reopens.
    const h = harness();
    const code = openHost(h);
    h.connect('early', ROLE_CLIENT);
    h.send('early', { type: 'join', code: code.suffix });
    const seenBefore = h.frames('early').length;

    h.send('host-1', { type: 'host-admission', state: 'closed' });
    h.connect('late', ROLE_CLIENT);
    h.send('late', { type: 'join', code: code.suffix });
    expect(h.last('late', 'error')).toMatchObject({ reason: 'admission-closed' });
    expect(h.frames('early')).toHaveLength(seenBefore);
    expect(h.reg.snapshot()[0].peers).toBe(1);

    h.send('host-1', { type: 'host-admission', state: 'open' });
    h.send('late', { type: 'join', code: code.suffix });
    expect(h.last('late', 'joined')).toMatchObject({ peer: 'late', admission: 'open' });
    expect(h.reg.snapshot()[0].peers).toBe(2);
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

describe('fleet joining (issue #1114)', () => {
  /** A second ship host asking to join a fleet: role client, namespace server. */
  const fleetJoin = (h, connId, code) => {
    h.connect(connId, ROLE_CLIENT);
    return h.send(connId, { type: 'join', code, namespace: NAMESPACE_SERVER });
  };

  it('admits a second ship host on the privileged code', () => {
    const h = harness();
    const code = openHost(h, 'fleet-lead', NAMESPACE_SERVER);
    fleetJoin(h, 'ship-2', code.suffix);
    expect(h.last('ship-2', 'joined')).toMatchObject({ peer: 'ship-2', admission: 'open' });
    expect(h.last('fleet-lead', 'peer-joined')).toMatchObject({ peer: 'ship-2' });
  });

  it('composes a bare five-letter fleet code under the SERVER project', () => {
    // The sharpest case for the typed fallback: one suffix, two records, two
    // fields. Without a per-request namespace the fleet field would compose
    // the crew project and attach a ship host to a phone's ship.
    const script = [...letters('QUARK'), ...letters('QUARK')];
    let i = 0;
    const h = harness({ randomInt: () => script[i++] });
    const crew = openHost(h, 'crew-host', NAMESPACE_CLIENT);
    const fleet = openHost(h, 'fleet-lead', NAMESPACE_SERVER);
    expect(crew.suffix).toBe(fleet.suffix);

    fleetJoin(h, 'ship-2', 'quark');
    expect(h.last('fleet-lead', 'peer-joined')).toMatchObject({ peer: 'ship-2' });
    expect(h.last('crew-host', 'peer-joined')).toBeNull();

    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: 'quark' });
    expect(h.last('crew-host', 'peer-joined')).toMatchObject({ peer: 'phone' });
  });

  it('refuses a crew code typed into the fleet field, by type', () => {
    const h = harness();
    const crew = openHost(h, 'crew-host', NAMESPACE_CLIENT);
    fleetJoin(h, 'ship-2', crew.suffix);
    expect(h.last('ship-2', 'error')).toMatchObject({ request: 'join', reason: 'wrong-type' });
    // And the same answer for a whole pasted crew code, not only five letters.
    // The `namespace` on that frame is the FIELD's, which is what the shipped
    // joiner sends for either form — it used to send the namespace read out of
    // the code itself, so the asker echoed the record back at the service and
    // this check could never fire for a full code. The joiner now also refuses
    // the disagreement before dialling (tests/client/fleet-session.test.js);
    // this is the service holding the same line for anything that reaches it.
    fleetJoin(h, 'ship-3', crew.full);
    expect(h.last('ship-3', 'error')).toMatchObject({ reason: 'wrong-type' });
  });

  it('refuses a fleet code typed into the crew field, by type', () => {
    const h = harness();
    const fleet = openHost(h, 'fleet-lead', NAMESPACE_SERVER);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: fleet.suffix, namespace: NAMESPACE_CLIENT });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'wrong-type' });
    // A phone built before #1114 sends no namespace at all and gets the same
    // answer — the field is additive, not a behaviour switch.
    h.connect('old-phone', ROLE_CLIENT);
    h.send('old-phone', { type: 'join', code: fleet.suffix });
    expect(h.last('old-phone', 'error')).toMatchObject({ reason: 'wrong-type' });
  });

  it('keeps the fleet lead closable without disconnecting an admitted ship', () => {
    const h = harness();
    const code = openHost(h, 'fleet-lead', NAMESPACE_SERVER);
    fleetJoin(h, 'ship-2', code.suffix);
    h.send('fleet-lead', { type: 'host-admission', state: 'closed' });

    fleetJoin(h, 'ship-3', code.suffix);
    expect(h.last('ship-3', 'error')).toMatchObject({ reason: 'admission-closed' });
    expect(h.last('ship-2', 'closed')).toBeNull();
    expect(h.reg.snapshot().find((r) => r.namespace === NAMESPACE_SERVER).peers).toBe(1);

    h.send('fleet-lead', { type: 'host-admission', state: 'open' });
    h.send('ship-3', { type: 'join', code: code.suffix, namespace: NAMESPACE_SERVER });
    expect(h.last('ship-3', 'joined')).toBeTruthy();
  });

  it('relays signalling between two ship hosts exactly as it does to a phone', () => {
    const h = harness();
    const code = openHost(h, 'fleet-lead', NAMESPACE_SERVER);
    fleetJoin(h, 'ship-2', code.suffix);
    h.send('ship-2', { type: 'signal', payload: { sdp: 'offer' } });
    expect(h.last('fleet-lead', 'signal')).toMatchObject({ from: 'ship-2', payload: { sdp: 'offer' } });
    h.send('fleet-lead', { type: 'signal', to: 'ship-2', payload: { sdp: 'answer' } });
    expect(h.last('ship-2', 'signal')).toMatchObject({ from: 'fleet-lead', payload: { sdp: 'answer' } });
  });

  it('resolves a fleet code without attaching, for a check-before-committing step', () => {
    const h = harness();
    const code = openHost(h, 'fleet-lead', NAMESPACE_SERVER);
    h.connect('ship-2', ROLE_CLIENT);
    h.send('ship-2', { type: 'resolve', code: code.suffix, namespace: NAMESPACE_SERVER });
    expect(h.last('ship-2', 'resolved')).toMatchObject({
      namespace: NAMESPACE_SERVER,
      admission: 'open',
    });
    expect(h.reg.snapshot().find((r) => r.namespace === NAMESPACE_SERVER).peers).toBe(0);
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
  it('holds the record in grace rather than freeing it when the host socket merely dies (issue #1115)', () => {
    // The PeerJS-less "the record is really gone" story now only applies once
    // the reclaim grace window has actually run out (see the next test) or the
    // host deliberately says `host-close` (the test after that). A socket
    // that simply dies — a Durable Object hiccup, a phone radio killing a
    // backgrounded viewscreen tab's WS — holds the record instead.
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    const seenBefore = h.frames('phone').length;

    h.disconnect('host-1');
    // Nothing is said to an already-admitted joiner at the moment of loss —
    // its DataChannel is a direct link the registry knows nothing about, and
    // a signalling blip is not the record actually dying.
    expect(h.frames('phone')).toHaveLength(seenBefore);
    expect(h.reg.snapshot()).toHaveLength(1);

    // The suffix is still a perfectly good, findable record…
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'resolved')).toMatchObject({ admission: 'open' });
    // …but nobody can actually JOIN it: there is no live host to admit
    // anyone, and `unreachable` is the same retryable answer a genuine
    // service blip already gives a joiner's own backoff loop.
    h.connect('late', ROLE_CLIENT);
    h.send('late', { type: 'join', code: code.suffix });
    expect(h.last('late', 'error')).toMatchObject({ request: 'join', reason: 'unreachable' });
  });

  it('drops the record for good once the grace window runs out with nobody reclaiming it', () => {
    let clock = 1_000_000;
    const h = harness({ now: () => clock });
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    h.disconnect('host-1');

    clock += DATA.limits.reclaim_grace_seconds * 1000 + 1;
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'closed')).toMatchObject({ reason: 'host-gone' });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'unknown' });
    expect(h.reg.snapshot()).toHaveLength(0);
  });

  it('still drops the record for real and immediately on an EXPLICIT host-close', () => {
    // A deliberate teardown has nothing to reclaim — only a socket that
    // merely dies gets the grace hold above.
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });

    h.send('host-1', { type: 'host-close' });
    expect(h.last('phone', 'closed')).toMatchObject({ reason: 'host-gone' });
    expect(h.reg.snapshot()).toHaveLength(0);
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
      // Issue #1113's WebSocket game relay: how many of those peers the
      // service is carrying itself. A count, exactly like `peers`, and for the
      // same reason it carries no id.
      relayPeers: 0,
    });
    // The suffix IS the private client code. A view that carries it is a
    // diagnostics endpoint one route away from handing out every live session.
    expect(JSON.stringify(h.reg.snapshot())).not.toContain(code.suffix);
  });
});

describe('code reclaim (issue #1115)', () => {
  it('hands the reclaim secret to the host that minted the record, and to nobody else', () => {
    const h = harness();
    const code = openHost(h);
    expect(code.secret).toMatch(/^[0-9a-f]{32}$/);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    // Never leaks to a joiner, on either the join or the presence frame the
    // host receives about it.
    expect(h.last('phone', 'joined')).not.toHaveProperty('secret');
    expect(h.last('host-1', 'peer-joined')).not.toHaveProperty('secret');
    // Not repeated on a later `hosted` that is only an admission ACK, either
    // — the host is expected to remember it from the first frame.
    h.send('host-1', { type: 'host-admission', state: 'closed' });
    expect(h.last('host-1', 'hosted')).not.toHaveProperty('code.secret');
  });

  it('revives the exact same record when the original host presents the secret in time', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    // Admission state is this fleet/host's own, and must survive the round
    // trip untouched — a reclaim is not a re-registration from scratch.
    h.send('host-1', { type: 'host-admission', state: 'closed' });
    h.disconnect('host-1');

    h.connect('host-1b', ROLE_HOST);
    h.send('host-1b', {
      type: 'host-open',
      namespace: NAMESPACE_CLIENT,
      resume: { suffix: code.suffix, secret: code.secret },
    });
    const revived = h.last('host-1b', 'hosted');
    expect(revived).toMatchObject({
      code: { suffix: code.suffix, full: code.full },
      admission: 'closed',
    });
    // The secret itself survives a reclaim unchanged — only an explicit
    // rotation mints a new one.
    expect(revived.code.secret).toBe(code.secret);

    // And the record really is live again: a join now succeeds (admission
    // has to be reopened first — it was closed above, same as any ordinary
    // operator lever) and reaches the reviving connection as host.
    h.send('host-1b', { type: 'host-admission', state: 'open' });
    h.send('phone', { type: 'join', code: code.suffix });
    expect(h.last('host-1b', 'peer-joined')).toMatchObject({ peer: 'phone' });
  });

  it('burns the grace-held record on a wrong secret, and mints the presenter a fresh one', () => {
    const h = harness();
    const code = openHost(h);
    h.disconnect('host-1');

    h.connect('host-1b', ROLE_HOST);
    h.send('host-1b', {
      type: 'host-open',
      namespace: NAMESPACE_CLIENT,
      resume: { suffix: code.suffix, secret: 'not-the-right-secret-at-all-00' },
    });
    const fresh = h.last('host-1b', 'hosted');
    expect(fresh.code.suffix).not.toBe(code.suffix);

    // The old suffix is unknown immediately — burned, not merely still
    // waiting out its grace window.
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'unknown' });

    // And presenting the SAME wrong secret again is a second first-time host,
    // not a second bite at the same guess.
    h.connect('host-1c', ROLE_HOST);
    h.send('host-1c', {
      type: 'host-open',
      namespace: NAMESPACE_CLIENT,
      resume: { suffix: code.suffix, secret: 'not-the-right-secret-at-all-00' },
    });
    expect(h.last('host-1c', 'hosted').code.suffix).not.toBe(code.suffix);
    expect(h.last('host-1c', 'hosted').code.suffix).not.toBe(fresh.code.suffix);
  });

  it('falls through to an ordinary fresh mint rather than stealing a still-LIVE record', () => {
    // The secret only means something once its record has nobody hosting it.
    // A live record's own host would never need to resume in the first
    // place; this proves a stray/forged resume cannot hijack one out from
    // under a connection that is still there.
    const h = harness();
    const code = openHost(h);

    h.connect('impostor', ROLE_HOST);
    h.send('impostor', {
      type: 'host-open',
      namespace: NAMESPACE_CLIENT,
      resume: { suffix: code.suffix, secret: code.secret },
    });
    const minted = h.last('impostor', 'hosted');
    expect(minted.code.suffix).not.toBe(code.suffix);
    // The original record is completely undisturbed: still there, still
    // owned by its real host.
    expect(h.reg.snapshot()).toHaveLength(2);
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'resolved')).toMatchObject({ admission: 'open' });
  });

  it('ignores a resume naming a suffix nobody ever minted, and mints normally', () => {
    const h = harness();
    h.connect('host-1', ROLE_HOST);
    h.send('host-1', {
      type: 'host-open',
      namespace: NAMESPACE_CLIENT,
      resume: { suffix: 'ZZZZZ', secret: 'whatever-was-guessed-here-000000' },
    });
    const minted = h.last('host-1', 'hosted');
    expect(minted.code.suffix).toMatch(new RegExp(`^[${DATA.suffix.alphabet}]{5}$`));
  });

  it('keeps a reclaim secret to its own namespace — a crew record cannot revive under the fleet one', () => {
    const h = harness();
    const crew = openHost(h, 'crew-host', NAMESPACE_CLIENT);
    h.disconnect('crew-host');

    h.connect('fleet-host', ROLE_HOST);
    h.send('fleet-host', {
      type: 'host-open',
      namespace: NAMESPACE_SERVER,
      resume: { suffix: crew.suffix, secret: crew.secret },
    });
    const minted = h.last('fleet-host', 'hosted');
    expect(minted.code.namespace).toBe(NAMESPACE_SERVER);
    expect(minted.code.suffix).not.toBe(crew.suffix);
  });
});

describe('explicit rotation (issue #1115 AC2/AC3)', () => {
  it('mints this record\'s live host a brand-new code, invalidating the old one immediately', () => {
    const h = harness();
    const code = openHost(h);
    h.send('host-1', { type: 'rotate' });
    const rotated = h.last('host-1', 'hosted');
    expect(rotated.code.suffix).not.toBe(code.suffix);
    expect(rotated.code.secret).not.toBe(code.secret);
    expect(rotated.admission).toBe('open');

    // The OLD suffix is unknown from the very next frame — not a grace hold,
    // a real drop.
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'resolve', code: code.suffix });
    expect(h.last('phone', 'error')).toMatchObject({ reason: 'unknown' });
    // …and the NEW one is live and joinable, on the same connection.
    h.send('phone', { type: 'join', code: rotated.code.suffix });
    expect(h.last('host-1', 'peer-joined')).toMatchObject({ peer: 'phone' });
  });

  it('tells a peer still mid-signal on the old code that it is gone, without a spurious fault to the rotating host itself', () => {
    const h = harness();
    const code = openHost(h);
    h.connect('waiting', ROLE_CLIENT);
    h.send('waiting', { type: 'join', code: code.suffix });

    h.send('host-1', { type: 'rotate' });
    expect(h.last('waiting', 'closed')).toMatchObject({ reason: 'host-gone' });
    // The rotating host itself does NOT get dropRecord's ordinary
    // "your record is gone" — it asked for this, and that frame is what
    // createRendezvousHost's own onError treats as a service fault.
    expect(h.frames('host-1').filter((f) => f.type === 'error')).toHaveLength(0);
  });

  it('refuses to rotate a record this connection is not the live host of', () => {
    const h = harness();
    openHost(h);
    h.connect('impostor', ROLE_CLIENT);
    h.send('impostor', { type: 'rotate' });
    expect(h.last('impostor', 'error')).toMatchObject({ request: 'rotate', reason: 'not-hosting' });

    // Nor while merely grace-held — the disconnected original host has no
    // live connId left to ask with, and nobody else may ask on its behalf.
    h.disconnect('host-1');
    h.connect('bystander', ROLE_CLIENT);
    h.send('bystander', { type: 'rotate' });
    expect(h.last('bystander', 'error')).toMatchObject({ reason: 'not-hosting' });
  });

  it('touches nothing about another record — not another host\'s code, not another namespace (AC4)', () => {
    const h = harness();
    const crew = openHost(h, 'crew-host', NAMESPACE_CLIENT);
    const fleet = openHost(h, 'fleet-host', NAMESPACE_SERVER);
    const otherCrew = openHost(h, 'other-crew-host', NAMESPACE_CLIENT, { version: OTHER_RELEASE });
    h.send('fleet-host', { type: 'host-admission', state: 'closed' });

    h.send('crew-host', { type: 'rotate' });
    const rotated = h.last('crew-host', 'hosted');
    expect(rotated.code.suffix).not.toBe(crew.suffix);

    // The fleet record and the other release's crew record are completely
    // undisturbed by rotating a THIRD, unrelated one — same suffix, same
    // admission state, still exactly as resolvable as before.
    h.connect('checker', ROLE_CLIENT);
    h.send('checker', { type: 'resolve', code: fleet.suffix, namespace: NAMESPACE_SERVER });
    expect(h.last('checker', 'resolved')).toMatchObject({ admission: 'closed' });
    h.send('checker', {
      type: 'resolve',
      code: composeJoinCode({ project: CLIENT_PROJECT, version: OTHER_RELEASE, suffix: otherCrew.suffix }),
    });
    expect(h.last('checker', 'resolved')).toMatchObject({ admission: 'open' });
    expect(h.reg.snapshot()).toHaveLength(3);
  });

  it('mints a fresh secret, so the OLD one can no longer reclaim anything', () => {
    const h = harness();
    const code = openHost(h);
    h.send('host-1', { type: 'rotate' });
    h.disconnect('host-1');

    h.connect('host-1b', ROLE_HOST);
    h.send('host-1b', {
      type: 'host-open',
      namespace: NAMESPACE_CLIENT,
      resume: { suffix: code.suffix, secret: code.secret },
    });
    // The pre-rotation suffix was dropped for real by `rotate` itself, so
    // there is nothing grace-held left for the old secret to revive — this
    // is an ordinary fresh mint, same as resuming any other unknown suffix.
    expect(h.last('host-1b', 'hosted').code.suffix).not.toBe(code.suffix);
  });
});
