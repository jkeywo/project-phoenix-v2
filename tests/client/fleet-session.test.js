// Two ship hosts assembling a fleet, in process (issue #1114).
//
// The same shape as tests/client/rendezvous-transport.test.js's world: a fake
// WebSocket pair terminated by the REAL rendezvous registry, plus a fake
// RTCPeerConnection pair that links two DataChannels once SDP has crossed. So
// what is under test here is the whole path a second host actually takes —
// typed fleet code, service lookup, offer, compatibility handshake, host-mesh
// hello — with no sockets, no WebRTC and no worker.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { createFleetOwner, createFleetMember } from '../../gui/fleet-session.js';
import { createRegistry, ROLE_HOST, ROLE_CLIENT } from '../../worker-rendezvous/src/registry.js';
import { ADMISSION_CLOSED, ADMISSION_OPEN, asHostFrame } from '../../gui/host-mesh.js';
import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  setJoinCodeData,
  projectGuidFor,
  versionGuid,
  composeJoinCode,
} from '../../gui/join-code.js';
import { createRendezvousHost } from '../../gui/rendezvous-transport.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const DATA = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));
setJoinCodeData(DATA);

const MAX_SLOTS = DATA.limits.max_fleet_hosts;
const STAMP = '1/phoenix-base/1';
const settle = async () => {
  for (let i = 0; i < 60; i += 1) await Promise.resolve();
};

// ── Fakes (same construction as the transport suite's) ──────────────────────

function makeWorld() {
  const registry = createRegistry({ data: DATA });
  const sockets = new Map();
  let n = 0;
  const dispatch = (frames) => {
    for (const { to, frame } of frames) {
      const ws = sockets.get(to);
      if (ws && ws.onmessage) ws.onmessage({ data: JSON.stringify(frame) });
    }
  };
  function socket(url) {
    const id = `conn-${++n}`;
    const role = url.endsWith('/v1/host') ? ROLE_HOST : ROLE_CLIENT;
    const ws = {
      readyState: 1,
      onopen: null, onmessage: null, onerror: null, onclose: null,
      send(text) { dispatch(registry.receive(id, JSON.parse(text))); },
      close() {
        if (this.readyState === 3) return;
        this.readyState = 3;
        sockets.delete(id);
        dispatch(registry.disconnect(id));
        if (this.onclose) this.onclose();
      },
    };
    sockets.set(id, ws);
    queueMicrotask(() => {
      if (ws.onopen) ws.onopen();
      dispatch(registry.connect(id, role));
    });
    return ws;
  }
  return { registry, socket };
}

function makePeerFactory() {
  const offerers = new Map();
  const channels = [];
  let n = 0;
  function makeChannel(label, init, origin) {
    const ch = {
      label,
      init: init || {},
      origin,
      sent: [],
      readyState: 'connecting',
      onopen: null, onmessage: null, onclose: null, onerror: null,
      _remote: null,
      send(payload) {
        this.sent.push(payload);
        const remote = this._remote;
        queueMicrotask(() => { if (remote && remote.onmessage) remote.onmessage({ data: payload }); });
      },
      close() {
        if (this.readyState === 'closed') return;
        this.readyState = 'closed';
        if (this.onclose) this.onclose();
        const remote = this._remote;
        if (remote && remote.readyState !== 'closed') queueMicrotask(() => remote.close());
      },
    };
    channels.push(ch);
    return ch;
  }
  function link(offerer, answerer) {
    for (const local of offerer._channels) {
      const remote = makeChannel(local.label, local.init, 'answer');
      local._remote = remote;
      remote._remote = local;
      answerer._channels.push(remote);
      queueMicrotask(() => {
        local.readyState = 'open';
        remote.readyState = 'open';
        if (answerer.ondatachannel) answerer.ondatachannel({ channel: remote });
        if (remote.onopen) remote.onopen();
        if (local.onopen) local.onopen();
      });
    }
  }
  const factory = function peer() {
    const id = `pc-${++n}`;
    return {
      _id: id,
      _channels: [],
      localDescription: null,
      remoteDescription: null,
      onicecandidate: null,
      oniceconnectionstatechange: null,
      iceConnectionState: 'checking',
      ondatachannel: null,
      createDataChannel(label, init) {
        const ch = makeChannel(label, init, 'offer');
        this._channels.push(ch);
        return ch;
      },
      async createOffer() { offerers.set(id, this); return { type: 'offer', peer: id }; },
      async createAnswer() { return { type: 'answer', peer: id }; },
      async setLocalDescription(d) { this.localDescription = d; },
      async setRemoteDescription(d) {
        this.remoteDescription = d;
        if (d.type === 'offer') link(offerers.get(d.peer), this);
      },
      async addIceCandidate() {},
      close() { for (const c of this._channels) c.close(); },
    };
  };
  factory.channels = channels;
  return factory;
}

// ── Harness ─────────────────────────────────────────────────────────────────

/** A fleet lead on a fresh world, plus everything a test wants to look at. */
async function leadOn(world, factories, opts = {}) {
  const rosters = [];
  let code = null;
  const fleet = createFleetOwner({
    base: 'https://rendezvous.test',
    factories,
    maxSlots: MAX_SLOTS,
    checkStamp: () => ({ ok: true }),
    name: 'Lead',
    onCode: (c) => { code = c; },
    onRoster: (r) => rosters.push(r),
    ...opts,
  });
  await settle();
  return { fleet, rosters, get code() { return code; } };
}

/** A second ship host typing a fleet code. */
async function memberOn(world, factories, code, opts = {}) {
  const rosters = [];
  const refusals = [];
  const statuses = [];
  const member = createFleetMember({
    base: 'https://rendezvous.test',
    data: DATA,
    code,
    stamp: STAMP,
    factories,
    name: 'Two',
    onRoster: (r) => rosters.push(r),
    onError: (reason, detail) => refusals.push({ reason, detail }),
    onStatus: (s) => statuses.push(s),
    ...opts,
  });
  await settle();
  return { member, rosters, refusals, statuses };
}

/** One world, one shared peer factory — both ends must see the same links. */
async function fleetOf(opts = {}) {
  const world = makeWorld();
  const factories = { socket: world.socket, peer: makePeerFactory() };
  const lead = await leadOn(world, factories, opts);
  return { world, factories, lead };
}

const lastRoster = (side) => side.rosters[side.rosters.length - 1];

// ── Tests ───────────────────────────────────────────────────────────────────

describe('opening a fleet', () => {
  it('mints one privileged code in the server namespace', async () => {
    const { lead } = await fleetOf();
    expect(lead.code.namespace).toBe(NAMESPACE_SERVER);
    expect(lead.code.project).toBe(projectGuidFor(NAMESPACE_SERVER, DATA));
    expect(lead.code.suffix).toHaveLength(DATA.suffix.length);
    expect(lead.fleet.code.full).toBe(lead.code.full);
  });

  it('is a SECOND record beside the ship\'s own crew code, not a replacement', async () => {
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    // The ordinary crew registration this page already makes.
    let crewCode = null;
    createRendezvousHost({
      base: 'https://rendezvous.test',
      factories,
      onCode: (c) => { crewCode = c; },
    });
    const lead = await leadOn(world, factories);
    expect(crewCode.namespace).toBe(NAMESPACE_CLIENT);
    expect(lead.code.namespace).toBe(NAMESPACE_SERVER);
    expect(world.registry.snapshot().map((r) => r.namespace).sort())
      .toEqual([NAMESPACE_CLIENT, NAMESPACE_SERVER]);
  });

  it('starts with only the lead in the roster', async () => {
    const { lead } = await fleetOf({ ship: { template_path: 'destroyer.toml' } });
    const roster = lastRoster(lead);
    expect(roster.slots).toHaveLength(1);
    expect(roster.slots[0]).toMatchObject({ id: 'slot-1', owner: true, connected: true });
    expect(roster.admission).toBe(ADMISSION_OPEN);
    expect(roster.frozen).toBe(false);
  });
});

describe('admitting a second ship host', () => {
  it('gives it its own slot, and both hosts see the same fleet', async () => {
    const { factories, world, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix.toLowerCase());

    expect(two.member.slot).toBe('slot-2');
    expect(lastRoster(two).slots.map((s) => s.id)).toEqual(['slot-1', 'slot-2']);
    expect(lastRoster(lead).slots.map((s) => s.id)).toEqual(['slot-1', 'slot-2']);
    expect(lastRoster(lead)).toEqual(lastRoster(two));
    expect(two.statuses).toContain('ready');
  });

  it('carries the joining host\'s hull into the lead\'s roster', async () => {
    const { factories, world, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix, {
      ship: { template_path: 'cruiser.toml', name: 'Ironveil' },
    });
    expect(lastRoster(lead).slots[1].ship).toEqual({
      template_path: 'cruiser.toml',
      name: 'Ironveil',
    });
    // …and a later change reaches it too, which is the half mission start freezes.
    two.member.update({ ship: { template_path: 'destroyer.toml' }, ready: true });
    await settle();
    expect(lastRoster(lead).slots[1]).toMatchObject({
      ready: true,
      ship: { template_path: 'destroyer.toml' },
    });
  });

  it('speaks only host-mesh frames — no Identify, no crew protocol', async () => {
    const { factories, world, lead } = await fleetOf();
    await memberOn(world, factories, lead.code.suffix);

    const reliable = factories.peer.channels.filter((c) => c.label === 'reliable');
    const payloads = reliable.flatMap((c) => c.sent).map((p) => JSON.parse(p));
    expect(payloads.length).toBeGreaterThan(0);
    // The transport-plane handshake, then host-mesh frames. Nothing that a
    // crew decoder would look at twice.
    for (const p of payloads) {
      const transportPlane = p.type === 'JoinHandshake' || p.type === 'JoinAccepted';
      expect(transportPlane || asHostFrame(p) !== null, JSON.stringify(p)).toBe(true);
    }
    expect(payloads.some((p) => p.type === 'Identify')).toBe(false);
  });

  it('refuses a host on another build, and never seats it', async () => {
    const { factories, world, lead } = await fleetOf({
      checkStamp: () => ({ ok: false, code: 'content-epoch-mismatch', detail: 'host 1, peer 2' }),
    });
    const two = await memberOn(world, factories, lead.code.suffix);

    expect(two.member.slot).toBeNull();
    expect(two.refusals.map((r) => r.reason)).toContain('content-epoch-mismatch');
    expect(lastRoster(lead).slots).toHaveLength(1);
  });

  it('refuses when it cannot check the build at all, rather than admitting', async () => {
    // The deliberate inversion of the crew path's default: a host that cannot
    // ask must not seat another authoritative simulation.
    const { factories, world, lead } = await fleetOf({ checkStamp: undefined });
    const two = await memberOn(world, factories, lead.code.suffix);
    expect(two.member.slot).toBeNull();
    expect(two.refusals.map((r) => r.reason)).toContain('client-stamp-missing');
  });
});

describe('typed-code refusal', () => {
  it('refuses a crew code entered into the fleet field', async () => {
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    let crewCode = null;
    createRendezvousHost({
      base: 'https://rendezvous.test',
      factories,
      onCode: (c) => { crewCode = c; },
    });
    await settle();

    const two = await memberOn(world, factories, crewCode.suffix);
    expect(two.member.slot).toBeNull();
    expect(two.refusals.map((r) => r.reason)).toContain('wrong-type');
  });

  it('refuses a code belonging to no Phoenix namespace before dialling anything', async () => {
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const stray = composeJoinCode({
      project: '00000000-0000-4000-8000-000000000000',
      version: versionGuid(DATA),
      suffix: 'QUARK',
    });
    const two = await memberOn(world, factories, stray);
    expect(two.refusals.map((r) => r.reason)).toContain('unknown-project');
  });
});

describe('closing and reopening admission', () => {
  it('refuses a new host, leaves an admitted one alone, and admits again on reopen', async () => {
    const { factories, world, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix);
    expect(two.member.slot).toBe('slot-2');

    lead.fleet.setAdmission(ADMISSION_CLOSED);
    await settle();
    expect(lastRoster(lead).admission).toBe(ADMISSION_CLOSED);
    // The admitted host was TOLD, not dropped.
    expect(lastRoster(two).admission).toBe(ADMISSION_CLOSED);
    expect(two.member.slot).toBe('slot-2');

    const three = await memberOn(world, factories, lead.code.suffix);
    expect(three.member.slot).toBeNull();
    expect(three.refusals.map((r) => r.reason)).toContain('admission-closed');
    expect(lastRoster(lead).slots).toHaveLength(2);

    lead.fleet.setAdmission(ADMISSION_OPEN);
    await settle();
    const four = await memberOn(world, factories, lead.code.suffix);
    expect(four.member.slot).toBe('slot-3');
    expect(lastRoster(lead).slots).toHaveLength(3);
  });

  it('does not disturb an admitted host\'s ability to change its own hull', async () => {
    const { factories, world, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix);
    lead.fleet.setAdmission(ADMISSION_CLOSED);
    await settle();
    two.member.update({ ready: true });
    await settle();
    expect(lastRoster(lead).slots[1].ready).toBe(true);
  });
});

describe('mission start freezes the fleet', () => {
  it('refuses a further host with a recovery reason, not a closure one', async () => {
    const { factories, world, lead } = await fleetOf();
    await memberOn(world, factories, lead.code.suffix);
    lead.fleet.freeze();
    await settle();

    expect(lastRoster(lead).frozen).toBe(true);
    const three = await memberOn(world, factories, lead.code.suffix);
    expect(three.member.slot).toBeNull();
    expect(three.refusals.map((r) => r.reason)).toContain('recovery-only');
  });

  it('refuses a loadout change from a host already in the fleet', async () => {
    const { factories, world, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix, {
      ship: { template_path: 'cruiser.toml' },
    });
    lead.fleet.freeze();
    await settle();

    two.member.update({ ship: { template_path: 'destroyer.toml' } });
    await settle();
    expect(lastRoster(lead).slots[1].ship).toEqual({ template_path: 'cruiser.toml' });
    expect(two.refusals.map((r) => r.reason)).toContain('recovery-only');
  });

  it('refuses the lead\'s own loadout change too', async () => {
    const { lead } = await fleetOf({ ship: { template_path: 'destroyer.toml' } });
    expect(lead.fleet.update({ ship: { template_path: 'cruiser.toml' } })).toBe(true);
    lead.fleet.freeze();
    expect(lead.fleet.update({ ship: { template_path: 'courier.toml' } })).toBe(false);
    expect(lastRoster(lead).slots[0].ship).toEqual({ template_path: 'cruiser.toml' });
  });

  it('leaves the code resolvable so a later claim can reach this host at all', async () => {
    // p2p-fixed-host-slot-recovery: the server code stays a recovery
    // capability across mission start, and "closing new-host admission does
    // not invalidate it for a known disconnected slot". So the SERVICE gate is
    // released at the freeze even when the operator had closed it — otherwise
    // a pre-start closure would be a permanent lockout of the replacement
    // machine, and #1120 would have to undo this rather than extend it.
    const { factories, world, lead } = await fleetOf();
    lead.fleet.setAdmission(ADMISSION_CLOSED);
    await settle();
    expect(world.registry.snapshot()[0].admission).toBe(ADMISSION_CLOSED);

    lead.fleet.freeze();
    await settle();
    expect(world.registry.snapshot()[0].admission).toBe(ADMISSION_OPEN);

    // It reaches the host, and the host — not the service — is what refuses it.
    const claim = await memberOn(world, factories, lead.code.suffix);
    expect(claim.refusals.map((r) => r.reason)).toEqual(['recovery-only']);
  });

  it('cannot be thawed by reopening admission', async () => {
    const { factories, world, lead } = await fleetOf();
    lead.fleet.freeze();
    lead.fleet.setAdmission(ADMISSION_OPEN);
    await settle();
    expect(lastRoster(lead).admission).toBe(ADMISSION_CLOSED);
    const two = await memberOn(world, factories, lead.code.suffix);
    expect(two.member.slot).toBeNull();
  });

  it('keeps a departed host\'s slot for recovery once frozen, and drops it before', async () => {
    const { factories, world, lead } = await fleetOf();
    const early = await memberOn(world, factories, lead.code.suffix);
    early.member.close();
    await settle();
    expect(lastRoster(lead).slots).toHaveLength(1);

    const two = await memberOn(world, factories, lead.code.suffix);
    expect(two.member.slot).toBe('slot-3');
    lead.fleet.freeze();
    two.member.close();
    await settle();
    const slots = lastRoster(lead).slots;
    expect(slots).toHaveLength(2);
    expect(slots[1]).toMatchObject({ id: 'slot-3', connected: false });
  });
});

describe('fleet capacity', () => {
  it('refuses past the authored number of ships', async () => {
    const { factories, world, lead } = await fleetOf();
    for (let i = 0; i < MAX_SLOTS - 1; i += 1) {
      const m = await memberOn(world, factories, lead.code.suffix);
      expect(m.member.slot, `member ${i}`).toBeTruthy();
    }
    expect(lastRoster(lead).slots).toHaveLength(MAX_SLOTS);
    const overflow = await memberOn(world, factories, lead.code.suffix);
    expect(overflow.member.slot).toBeNull();
    expect(overflow.refusals.map((r) => r.reason)).toContain('fleet-full');
  });
});
