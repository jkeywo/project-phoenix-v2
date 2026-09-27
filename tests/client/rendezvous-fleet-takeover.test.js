import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { createRegistry, ROLE_HOST, ROLE_CLIENT, RENDEZVOUS_PROTOCOL } from '../../worker-rendezvous/src/registry.js';
import { setJoinCodeData } from '../../gui/join-code.js';

const data = JSON.parse(readFileSync(new URL('../../assets/join/join-codes.json', import.meta.url)));
setJoinCodeData(data);
function fixture({ namespace = 'server', relay = true } = {}) {
  let time = 1000;
  const registry = createRegistry({ data, now: () => time });
  const inbox = new Map();
  const collect = frames => { for (const item of frames) { if (!inbox.has(item.to)) inbox.set(item.to, []); inbox.get(item.to).push(item.frame); } return frames; };
  const connect = (id, role = ROLE_CLIENT) => collect(registry.connect(id, role));
  const send = (id, frame) => collect(registry.receive(id, { v: RENDEZVOUS_PROTOCOL, ...frame }));
  const last = (id, type) => (inbox.get(id) || []).filter(frame => frame.type === type).at(-1);
  const disconnect = id => collect(registry.disconnect(id));
  connect('owner', ROLE_HOST); send('owner', { type: 'host-open', namespace });
  const code = last('owner', 'hosted').code;
  for (const id of ['ship', 'gm', 'guest']) {
    connect(id); send(id, { type: 'join', code: code.full, namespace });
    if (relay) send(id, { type: 'relay-open' });
  }
  const registration = { type: 'fleet-successors', frozen: true, epoch: 0, owner_slot: 1,
    members: [{ peer: 'gm', slot: 3 }, { peer: 'ship', slot: 2 }] };
  const arm = () => send('owner', registration);
  const proof = id => { const frame = last(id, 'fleet-capability'); return { epoch: frame.epoch, slot: frame.slot, capability: frame.capability }; };
  const request = id => send(id, { type: 'fleet-takeover', ...proof(id) });
  const bind = (id = 'successor') => {
    const grant = last('ship', 'fleet-takeover-grant'); connect(id, ROLE_HOST);
    return send(id, { type: 'host-open', namespace: 'server', takeover: grant, transports: ['ws-relay'] });
  };
  return { registry, connect, send, last, disconnect, code, registration, arm, proof, request, bind, advance: ms => { time += ms; } };
}

describe('frozen fleet rendezvous ownership continuity', () => {
  it('requires the current fleet owner and exact bounded distinct joined members', () => {
    const h = fixture();
    for (const sender of ['ship', 'guest']) {
      h.send(sender, h.registration); expect(h.last(sender, 'error').reason).toBe('not-hosting-fleet');
    }
    for (const members of [[{ peer: 'absent', slot: 2 }], [{ peer: 'ship', slot: 1 }],
      [{ peer: 'ship', slot: 2 }, { peer: 'gm', slot: 2 }], [{ peer: 'ship', slot: 2 }, { peer: 'ship', slot: 3 }]]) {
      h.send('owner', { ...h.registration, members }); expect(h.last('owner', 'error').reason).toBe('invalid-member');
    }
    h.send('owner', { ...h.registration, frozen: false }); expect(h.last('owner', 'error').reason).toBe('malformed');
    h.send('owner', { ...h.registration, members: Array(100).fill({ peer: 'ship', slot: 2 }) });
    expect(h.last('owner', 'error').reason).toBe('malformed');
    const crew = fixture({ namespace: 'client' }); crew.arm();
    expect(crew.last('owner', 'error').reason).toBe('not-hosting-fleet');
  });

  it('issues distinct private proofs only to authorized members and keeps registration idempotent', () => {
    const h = fixture(); const frames = h.arm();
    const capabilities = ['owner', 'ship', 'gm'].map(id => h.proof(id).capability);
    expect(new Set(capabilities).size).toBe(3);
    for (const capability of capabilities) expect(capability).toMatch(/^[0-9a-f]{32}$/);
    expect(capabilities).not.toContain(h.code.secret);
    expect(frames.some(frame => frame.to === 'guest')).toBe(false);
    const before = h.proof('ship'); h.arm(); expect(h.proof('ship')).toEqual(before);
    h.send('owner', { ...h.registration, members: [{ peer: 'ship', slot: 7 }] });
    expect(h.last('owner', 'error').reason).toBe('already-configured');
    for (const secret of [...capabilities, h.code.secret]) expect(JSON.stringify(h.registry.snapshot())).not.toContain(secret);
  });

  it('requires loss and a valid proof, preserves delegated sockets, and elects the lowest live slot', () => {
    const h = fixture(); h.arm(); h.request('ship');
    expect(h.last('ship', 'error').reason).toBe('host-present');
    const loss = h.disconnect('owner');
    for (const id of ['ship', 'gm']) {
      const types = loss.filter(item => item.to === id).map(item => item.frame.type);
      expect(types).toEqual(['fleet-owner-lost', 'relay-closed']);
    }
    expect(loss.filter(item => item.to === 'guest').map(item => item.frame.type)).toEqual(['relay-closed', 'closed']);
    h.send('gm', { type: 'fleet-takeover', epoch: 0, capability: h.proof('ship').capability });
    expect(h.last('gm', 'error').reason).toBe('forbidden-takeover');
    h.request('gm'); expect(h.last('gm', 'fleet-takeover-wait')).toMatchObject({ epoch: 0, slot: 2 });
    h.request('ship'); const grant = h.last('ship', 'fleet-takeover-grant');
    expect(grant).toMatchObject({ epoch: 1, slot: 2, suffix: h.code.suffix });
    expect(grant.capability).not.toBe(h.proof('ship').capability);
    h.request('ship'); expect(h.last('ship', 'fleet-takeover-grant')).toEqual(grant);
  });

  it('does not auto-promote on signalling loss and skips a disconnected lower slot', () => {
    const h = fixture({ relay: false }); h.arm();
    const loss = h.disconnect('owner');
    expect(loss.every(item => item.frame.type === 'fleet-owner-lost')).toBe(true);
    expect(h.last('ship', 'fleet-takeover-grant')).toBeUndefined();
    h.send('ship', { type: 'relay-open' });
    expect(h.last('ship', 'error').reason).toBe('unreachable');
    h.disconnect('ship'); h.request('gm');
    expect(h.last('gm', 'fleet-takeover-grant')).toMatchObject({ epoch: 1, slot: 3 });
  });

  it('reserves takeover against old reclaim and binds the same code with a fresh owner secret', () => {
    const h = fixture(); h.arm(); h.disconnect('owner'); h.request('ship');
    h.connect('old-owner', ROLE_HOST);
    h.send('old-owner', { type: 'host-open', namespace: 'server', resume: h.code });
    expect(h.last('old-owner', 'error').reason).toBe('takeover-pending');
    const bound = h.bind(); const hosted = h.last('successor', 'hosted');
    expect(hosted.code.full).toBe(h.code.full); expect(hosted.code.secret).not.toBe(h.code.secret);
    expect(hosted.fleet).toEqual({ ...h.proof('ship'), epoch: 1 });
    expect(h.registry.snapshot()).toHaveLength(1);
    for (const frame of bound.filter(item => item.to !== 'successor')) {
      expect(frame.frame).toMatchObject({ type: 'fleet-owner-changed', epoch: 1, slot: 2 });
      expect(JSON.stringify(frame)).not.toContain('capability'); expect(JSON.stringify(frame)).not.toContain('secret');
    }
    h.send('old-owner', { type: 'host-open', namespace: 'server', resume: h.code });
    expect(h.last('old-owner', 'error').reason).toBe('host-present');
    h.connect('duplicate-owner', ROLE_HOST);
    h.send('duplicate-owner', { type: 'host-open', namespace: 'server', takeover: h.last('ship', 'fleet-takeover-grant') });
    expect(h.last('duplicate-owner', 'error').reason).toBe('forbidden-takeover');
    expect(h.registry.snapshot()).toHaveLength(1);
  });

  it('rejoins a proven frozen slot through closed admission without displacing a connected holder', () => {
    const h = fixture(); h.arm(); h.send('owner', { type: 'host-admission', state: 'closed' });
    h.disconnect('owner'); h.request('ship'); h.bind();
    const join = { type: 'join', code: h.code.full, namespace: 'server', continuation: { ...h.proof('gm'), epoch: 1 } };
    h.connect('replacement-a'); h.send('replacement-a', join);
    expect(h.last('replacement-a', 'error').reason).toBe('slot-connected');
    h.disconnect('gm'); h.send('replacement-a', join);
    expect(h.last('replacement-a', 'joined')).toMatchObject({ continuation: { epoch: 1, slot: 3 }, transports: ['ws-relay'] });
    const notification = h.last('successor', 'peer-joined');
    expect(notification).toMatchObject({ peer: 'replacement-a', continuation: { epoch: 1, slot: 3 } });
    expect(JSON.stringify(notification)).not.toContain('capability');
    h.connect('replacement-b'); h.send('replacement-b', join);
    expect(h.last('replacement-b', 'error').reason).toBe('slot-connected');
    h.send('replacement-a', { type: 'relay-open' });
    expect(h.last('successor', 'relay-peer').continuation).toEqual({ epoch: 1, slot: 3 });
    h.send('replacement-a', { type: 'relay', class: 'reliable', payload: 'continued-state' });
    expect(h.last('successor', 'relay')).toMatchObject({ from: 'replacement-a', payload: 'continued-state' });
    h.connect('unrelated'); h.send('unrelated', { type: 'join', code: h.code.full, namespace: 'server' });
    expect(h.last('unrelated', 'error').reason).toBe('admission-closed');
  });

  it('authenticates epoch queries and lets a stale continuation discover the current owner', () => {
    const h = fixture(); h.arm(); h.disconnect('owner'); h.request('ship'); h.bind(); h.disconnect('gm');
    h.connect('gm-new');
    const request = { code: h.code.full, namespace: 'server', continuation: h.proof('gm') };
    h.send('gm-new', { type: 'join', ...request });
    expect(h.last('gm-new', 'error').reason).toBe('stale-fleet-epoch');
    expect(h.last('gm-new', 'fleet-state')).toMatchObject({ epoch: 1, owner_slot: 2, available: true });
    h.send('gm-new', { type: 'fleet-state', ...request });
    expect(h.last('gm-new', 'fleet-state')).toMatchObject({ epoch: 1, owner_slot: 2 });
    h.send('guest', { type: 'fleet-state', ...request, continuation: { slot: 3, capability: '0'.repeat(32) } });
    expect(h.last('guest', 'error').reason).toBe('forbidden-continuation');
  });

  it('preserves ordinary owner reclaim before reservation and refuses destructive wrong guesses', () => {
    const h = fixture(); h.arm(); h.disconnect('owner');
    h.connect('wrong', ROLE_HOST); h.send('wrong', { type: 'host-open', namespace: 'server', resume: { ...h.code, secret: '0'.repeat(32) } });
    expect(h.last('wrong', 'error').reason).toBe('forbidden-resume'); expect(h.registry.snapshot()).toHaveLength(1);
    h.connect('same-owner', ROLE_HOST); h.send('same-owner', { type: 'host-open', namespace: 'server', resume: h.code });
    expect(h.last('same-owner', 'hosted').fleet).toEqual(h.proof('owner'));
    expect(h.last('ship', 'fleet-owner-changed')).toMatchObject({ epoch: 0, slot: 1 });
    h.request('ship'); expect(h.last('ship', 'error').reason).toBe('host-present');
  });

  it('expires reserved grants at the original grace deadline and never mints on failed takeover', () => {
    const h = fixture(); h.arm(); h.disconnect('owner'); h.request('ship');
    h.advance((data.limits.reclaim_grace_seconds + 1) * 1000); h.bind();
    expect(h.last('successor', 'error').reason).toBe('forbidden-takeover');
    expect(h.registry.snapshot()).toHaveLength(0);
  });

  it('does not turn explicit teardown into takeover and meters capability probes', () => {
    const h = fixture(); h.arm(); h.send('owner', { type: 'host-close' }); h.request('ship');
    expect(h.last('ship', 'error').reason).toBe('forbidden-takeover'); expect(h.registry.snapshot()).toHaveLength(0);
    const active = fixture(); active.arm(); active.disconnect('owner');
    for (let i = 0; i <= data.limits.max_lookups_per_connection; i++) active.request('gm');
    expect(active.last('gm', 'error').reason).toBe('too-many-attempts');
  });
});
