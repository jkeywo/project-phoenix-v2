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
    // A draw that always returns 0 would mint AAAAA every time; the registry
    // must collision-check and move on.
    const draws = [];
    const h = harness({
      randomInt: () => (draws.length++ < DATA.suffix.length ? 0 : 1),
    });
    const first = openHost(h, 'host-1');
    const second = openHost(h, 'host-2');
    expect(second.suffix).not.toBe(first.suffix);
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
    const code = openHost(h, 'old-host', NAMESPACE_CLIENT, { version: 'older-release-guid' });
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
    const old = openHost(h, 'old-host', NAMESPACE_CLIENT, { version: 'older-release-guid' });
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
    h.send('phone', { type: 'join', code: code.suffix, stamp: '1/phoenix-base/1' });
    expect(h.last('phone', 'joined')).toMatchObject({ peer: 'phone', admission: 'open' });
    expect(h.last('host-1', 'peer-joined')).toMatchObject({ peer: 'phone', stamp: '1/phoenix-base/1' });
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

  it('relays the host stamp as advisory metadata on resolve and join', () => {
    const h = harness();
    const code = openHost(h, 'host-1', NAMESPACE_CLIENT, { stamp: '1/phoenix-base/7' });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: code.suffix });
    expect(h.last('phone', 'joined').host_stamp).toBe('1/phoenix-base/7');
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

  it('keeps a code private — the diagnostics snapshot names no peer and no stamp', () => {
    const h = harness();
    openHost(h, 'host-1', NAMESPACE_CLIENT, { stamp: '1/phoenix-base/1' });
    h.connect('phone', ROLE_CLIENT);
    h.send('phone', { type: 'join', code: h.last('host-1', 'hosted').code.suffix });
    const [row] = h.reg.snapshot();
    expect(row).toEqual({
      namespace: NAMESPACE_CLIENT,
      version: VERSION,
      suffix: expect.any(String),
      admission: 'open',
      peers: 1,
    });
  });
});
