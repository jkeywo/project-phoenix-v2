// Two ship hosts assembling a fleet, in process (issue #1114).
//
// The same shape as tests/client/rendezvous-transport.test.js's world: a fake
// WebSocket pair terminated by the REAL rendezvous registry, plus a fake
// RTCPeerConnection pair that links two DataChannels once SDP has crossed. So
// what is under test here is the whole path a second host actually takes —
// typed fleet code, service lookup, offer, compatibility handshake, host-mesh
// hello — with no sockets, no WebRTC and no worker.

import { describe, it, expect, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { createFleetOwner, createFleetMember } from '../../gui/fleet-session.js';
import { createRegistry, ROLE_HOST, ROLE_CLIENT } from '../../worker-rendezvous/src/registry.js';
import {
  ADMISSION_CLOSED,
  ADMISSION_OPEN,
  HOST_FRAME_HOST_LOSS,
  HOST_FRAME_TICK,
  asHostFrame,
  encodeHostFrame,
  helloFrame,
  simulationFrame,
} from '../../gui/host-mesh.js';
import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  setJoinCodeData,
  projectGuidFor,
  versionGuid,
  composeJoinCode,
} from '../../gui/join-code.js';
import { createRendezvousHost, createRendezvousJoiner } from '../../gui/rendezvous-transport.js';

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
  const notices = [];
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
    onRefusedSlot: (reason, detail) => notices.push({ reason, detail }),
    onStatus: (s) => statuses.push(s),
    ...opts,
  });
  await settle();
  return { member, rosters, refusals, notices, statuses };
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

describe('rotating the fleet code (issue #1115)', () => {
  it('is a pure passthrough onto the underlying host\'s rotate()', async () => {
    const { world, lead } = await fleetOf();
    const before = lead.code.suffix;
    lead.fleet.rotate();
    await settle();
    expect(lead.code.suffix).not.toBe(before);
    // The old suffix is gone from the registry; the new one is live.
    expect(world.registry.snapshot().map((r) => r.admission)).toEqual([ADMISSION_OPEN]);
  });

  it('lets a second ship host join on the fleet code that rotation actually produced', async () => {
    const { world, factories, lead } = await fleetOf();
    lead.fleet.rotate();
    await settle();
    const two = await memberOn(world, factories, lead.code.suffix.toLowerCase());
    expect(two.member.slot).toBeTruthy();
    expect(lastRoster(two).slots).toHaveLength(2);
  });

  it('does not evict a ship host already admitted on the code that just rotated', async () => {
    const { world, factories, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix.toLowerCase());
    expect(two.member.slot).toBeTruthy();

    lead.fleet.rotate();
    await settle();

    // The member's own direct link never touched the rendezvous service to
    // begin with (issue #1112's two-planes property, shared by the fleet
    // link) — its slot and roster are untouched by a code change it never
    // has to hear about.
    expect(two.refusals).toEqual([]);
    expect(two.member.slot).toBeTruthy();
    expect(lastRoster(lead).slots).toHaveLength(2);
  });

  it('does not touch the fleet\'s roster, freeze or admission state — only the letters', async () => {
    const { lead } = await fleetOf();
    lead.fleet.setAdmission(ADMISSION_CLOSED);
    lead.fleet.rotate();
    await settle();
    const roster = lastRoster(lead);
    expect(roster.admission).toBe(ADMISSION_CLOSED);
    expect(roster.frozen).toBe(false);
    expect(roster.slots).toHaveLength(1);
  });

  it('rotates even while the fleet is frozen — the roster stays locked, only the code moves', async () => {
    // AC2's mission-phase gate (Lobby/GameOver rotatable, InProgress refused)
    // is the CALLER's (server.html's codesRotatable), never fleet.frozen: the
    // freeze latch never resets once a mission has started, even back in
    // GameOver, and #1115 explicitly wants GameOver rotatable.
    const { lead } = await fleetOf();
    lead.fleet.freeze();
    const before = lead.code.suffix;
    lead.fleet.rotate();
    await settle();
    expect(lead.code.suffix).not.toBe(before);
    expect(lastRoster(lead).frozen).toBe(true);
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
  /** A world holding one crew record and one fleet record, and both codes. */
  async function bothNamespaces() {
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    let crewCode = null;
    createRendezvousHost({
      base: 'https://rendezvous.test',
      factories,
      onCode: (c) => { crewCode = c; },
    });
    const lead = await leadOn(world, factories);
    return { world, factories, lead, get crew() { return crewCode; } };
  }

  it('refuses a crew code entered into the fleet field', async () => {
    const { world, factories, crew } = await bothNamespaces();
    const two = await memberOn(world, factories, crew.suffix);
    expect(two.member.slot).toBeNull();
    expect(two.refusals.map((r) => r.reason)).toContain('wrong-type');
  });

  it('refuses a WHOLE crew code pasted into the fleet field', async () => {
    // The typed case above was never the dangerous one: a bare suffix is
    // composed under the field's own namespace, so it resolves the wrong
    // record or none. A FULL code carries its own project GUID, so the joiner
    // used to report the code's namespace instead of the field's — the asker
    // echoing the record back at the service, which then always agreed. The
    // one defence the branch rests on could not fire for the very form the
    // fleet panel hands the operator to paste.
    const { world, factories, crew } = await bothNamespaces();
    const two = await memberOn(world, factories, crew.full);
    expect(two.member.slot).toBeNull();
    expect(two.refusals.map((r) => r.reason)).toEqual(['wrong-type']);
    // Refused before anything was attached: the crew record has no peers.
    expect(world.registry.snapshot().map((r) => r.peers)).toEqual([0, 0]);
  });

  it('refuses a WHOLE fleet code pasted into a phone\'s crew field', async () => {
    // The same bug, in the direction that reaches a player: a phone pasting the
    // operator's full fleet code was attached to the FLEET record, admitted by
    // the lead's build check, and then hung — its Identify dropped by a decoder
    // that speaks another vocabulary, with no welcome and no error.
    const { world, factories, lead } = await bothNamespaces();
    const refusals = [];
    createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code: lead.code.full,
      // client.html passes no namespace at all, which is this default.
      namespace: NAMESPACE_CLIENT,
      stamp: STAMP,
      factories,
      onError: (reason) => refusals.push(reason),
    });
    await settle();
    expect(refusals).toEqual(['wrong-type']);
    expect(world.registry.snapshot().map((r) => r.peers)).toEqual([0, 0]);
  });

  it('still admits a whole code pasted into the field it belongs to', async () => {
    // The portable form has to keep working, or the refusal above is just a
    // ban on pasting.
    const { world, factories, lead } = await bothNamespaces();
    const two = await memberOn(world, factories, lead.code.full);
    expect(two.member.slot).toBe('slot-2');
    expect(two.refusals).toEqual([]);
  });

  it('refuses a code belonging to no Phoenix namespace before dialling anything', async () => {
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const stray = composeJoinCode({
      project: '00000000-0000-4000-8000-000000000000',
      version: versionGuid(DATA),
      suffix: 'QUARKING',
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

  it('refuses a loadout change from a host already in the fleet, without evicting it', async () => {
    const { factories, world, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix, {
      ship: { template_path: 'cruiser.toml' },
    });
    lead.fleet.freeze();
    await settle();

    two.member.update({ ship: { template_path: 'destroyer.toml' } });
    await settle();
    expect(lastRoster(lead).slots[1].ship).toEqual({ template_path: 'cruiser.toml' });
    // The refusal answers the PATCH, and says so. A member cannot be thrown
    // out of a fleet it belongs to for touching its own loadout: the terminal
    // callback — the one server.html answers with leaveFleet — never fires,
    // the slot is still held, and the link is still up.
    expect(two.notices.map((r) => r.reason)).toContain('recovery-only');
    expect(two.refusals).toEqual([]);
    expect(two.member.slot).toBe('slot-2');
    expect(lastRoster(lead).slots).toHaveLength(2);
    expect(lastRoster(lead).slots[1].connected).toBe(true);
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

  it('keeps that door open across a lost and regained registration', async () => {
    // The reconnect that used to undo the freeze's own rule. A replacement
    // registration starts in the service's default state, so whatever this
    // fleet last told the SERVICE has to be said again — and that is not the
    // model's `admission`, which the freeze sets to `closed` while deliberately
    // telling the service `open`. Replaying the model flag re-closed the
    // recovery door, which is exactly the permanent lockout of the replacement
    // machine that p2p-fixed-host-slot-recovery exists to prevent.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const hostSockets = [];
      const factories = {
        socket: (url) => {
          const s = world.socket(url);
          if (String(url).endsWith('/v1/host')) hostSockets.push(s);
          return s;
        },
        peer: makePeerFactory(),
      };
      const fleet = createFleetOwner({
        base: 'https://rendezvous.test',
        factories,
        maxSlots: MAX_SLOTS,
        checkStamp: () => ({ ok: true }),
      });
      await vi.advanceTimersByTimeAsync(0);

      // The operator closed admission before the mission; then it started.
      fleet.setAdmission(ADMISSION_CLOSED);
      fleet.freeze();
      await vi.advanceTimersByTimeAsync(0);
      expect(world.registry.snapshot()[0].admission).toBe(ADMISSION_OPEN);

      hostSockets[0].onerror();
      await vi.advanceTimersByTimeAsync(2_000);
      expect(hostSockets).toHaveLength(2);
      expect(world.registry.snapshot()).toHaveLength(1);
      expect(world.registry.snapshot()[0].admission).toBe(ADMISSION_OPEN);
    } finally {
      vi.useRealTimers();
    }
  });

  it('still re-asserts a pre-start closure across a lost registration', async () => {
    // The other half of the same rule: before the freeze, the model's closure
    // IS what the service was told, and a replacement record that silently
    // reopened would admit hosts the operator had shut out.
    vi.useFakeTimers();
    try {
      const world = makeWorld();
      const hostSockets = [];
      const factories = {
        socket: (url) => {
          const s = world.socket(url);
          if (String(url).endsWith('/v1/host')) hostSockets.push(s);
          return s;
        },
        peer: makePeerFactory(),
      };
      const fleet = createFleetOwner({
        base: 'https://rendezvous.test',
        factories,
        maxSlots: MAX_SLOTS,
        checkStamp: () => ({ ok: true }),
      });
      await vi.advanceTimersByTimeAsync(0);
      fleet.setAdmission(ADMISSION_CLOSED);
      await vi.advanceTimersByTimeAsync(0);

      hostSockets[0].onerror();
      await vi.advanceTimersByTimeAsync(2_000);
      expect(world.registry.snapshot()[0].admission).toBe(ADMISSION_CLOSED);
    } finally {
      vi.useRealTimers();
    }
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

describe('the running mission rides the same link (issue #1116)', () => {
  // What is under test is the ROUTING, not the frames: a `tick` or `digest`
  // frame's body is minted and read by Rust (`core::codec::encode_mesh_frame`),
  // and both halves of this module are supposed to ferry it without looking.

  const tickFrame = (from, tick) =>
    encodeHostFrame(simulationFrame(HOST_FRAME_TICK, {
      from,
      tick,
      ready_through: tick + 2,
      commands: [],
    }, tick));

  it('hands a simulation frame to the wasm boundary and NOT to the fleet model', async () => {
    const { factories, world, lead } = await fleetOf();
    const leadFrames = [];
    // A second lead wired with the callback the host page supplies.
    const wired = await leadOn(world, factories, {
      onSimulationFrame: (raw) => leadFrames.push(raw),
    });
    const two = await memberOn(world, factories, wired.code.suffix);
    expect(two.member.slot).toBe('slot-2');
    const before = lastRoster(wired);

    two.member.broadcast(tickFrame(2, 412));
    await settle();

    expect(leadFrames).toHaveLength(1);
    expect(asHostFrame(JSON.parse(leadFrames[0])).t).toBe(HOST_FRAME_TICK);
    expect(asHostFrame(JSON.parse(leadFrames[0])).d.ready_through).toBe(414);
    // The roster is untouched: a tick frame is not a roster edit, and the fleet
    // model must not have seen it at all.
    expect(lastRoster(wired)).toEqual(before);
    expect(lead.fleet.roster()).toBeTruthy();
  });

  it("carries the lead's own frames out to every member", async () => {
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories);
    const memberFrames = [];
    const two = await memberOn(world, factories, lead.code.suffix, {
      onSimulationFrame: (raw) => memberFrames.push(raw),
    });
    expect(two.member.slot).toBe('slot-2');

    lead.fleet.broadcast(tickFrame(1, 400));
    await settle();

    expect(memberFrames).toHaveLength(1);
    const decoded = asHostFrame(JSON.parse(memberFrames[0]));
    expect(decoded.t).toBe(HOST_FRAME_TICK);
    expect(decoded.d.from).toBe(1);
    // The envelope's tick stamp survives the round trip — it is what #1114 put
    // there for this, and #1118 replays against it.
    expect(decoded.tick).toBe(400);
  });

  it("relays a member's frame to its SIBLINGS, verbatim", async () => {
    // The transport is a star with the lead at the centre, so slot 2's input
    // reaches slot 3 only if the lead passes it on — and a lead that edited it
    // on the way would be speaking for a crew that is not its own.
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories);
    const two = await memberOn(world, factories, lead.code.suffix);
    const threeFrames = [];
    const three = await memberOn(world, factories, lead.code.suffix, {
      onSimulationFrame: (raw) => threeFrames.push(raw),
    });
    expect(three.member.slot).toBe('slot-3');

    const sent = tickFrame(2, 412);
    two.member.broadcast(sent);
    await settle();

    expect(threeFrames).toEqual([sent]);
  });

  it('reports a member host loss to the simulation once the mission has frozen (#1119)', async () => {
    // A ship host closing mid-mission is not a lobby slot going dark — the
    // simulation has to flip that ship to Backfill at an agreed tick. The owner
    // sees the socket close and hands the SLOT ORDINAL to the simulation, which
    // derives the tick and mints the host-loss frame the star relays onward.
    const lost = [];
    const { world, factories, lead } = await fleetOf({ onHostLost: (slot) => lost.push(slot) });
    const two = await memberOn(world, factories, lead.code.suffix.toLowerCase());
    expect(two.member.slot).toBe('slot-2');

    lead.fleet.freeze();
    await settle();

    two.member.close();
    await settle();

    expect(lost).toEqual([2]);
    // The slot is kept for #1120 recovery rather than removed, exactly as the
    // lobby-drop path already does after a freeze.
    expect(lastRoster(lead).slots.find((s) => s.id === 'slot-2')).toMatchObject({
      id: 'slot-2',
      connected: false,
    });
  });

  it('does NOT report a host loss for a drop before the mission starts (#1119)', async () => {
    // Before the freeze there is no running simulation to backfill anything in —
    // a host dropping is an ordinary lobby departure, and reporting it would
    // schedule a Backfill flip for a mission that has not begun.
    const lost = [];
    const { world, factories, lead } = await fleetOf({ onHostLost: (slot) => lost.push(slot) });
    const two = await memberOn(world, factories, lead.code.suffix.toLowerCase());
    expect(two.member.slot).toBe('slot-2');

    two.member.close();
    await settle();

    expect(lost).toEqual([]);
    // The slot is dropped from the lobby, as before the mission starts.
    expect(lastRoster(lead).slots).toHaveLength(1);
  });
});

describe('sender authentication and slot recovery (issue #1120)', () => {
  const tickFrame = (from, tick) =>
    encodeHostFrame(simulationFrame(HOST_FRAME_TICK, {
      from,
      tick,
      ready_through: tick + 2,
      commands: [],
    }, tick));

  it('tags a member\'s frame with the slot the connection was authenticated as', async () => {
    // The lead binds each admitted connection to its slot and hands that
    // authenticated slot to the wasm boundary beside the frame — the transport
    // half of the mesh-boundary authentication.
    const tagged = [];
    const { world, factories, lead } = await fleetOf({
      onSimulationFrame: (raw, authSlot) => tagged.push({ raw, authSlot }),
    });
    const two = await memberOn(world, factories, lead.code.suffix);
    expect(two.member.slot).toBe('slot-2');

    two.member.broadcast(tickFrame(2, 412));
    await settle();

    expect(tagged).toHaveLength(1);
    expect(tagged[0].authSlot).toBe(2);
  });

  it('a member tags every frame on its link with the LEAD\'s slot', async () => {
    // A member's single connection is to the lead, which delivers its own frames
    // and its relay of a sibling's; the member cannot re-authenticate a relayed
    // frame, so it tags every one with the lead's slot and Rust accepts it because
    // the lead already authenticated the origin at its ingress.
    const tagged = [];
    const { world, factories, lead } = await fleetOf();
    await memberOn(world, factories, lead.code.suffix, {
      onSimulationFrame: (raw, authSlot) => tagged.push({ raw, authSlot }),
    });

    lead.fleet.broadcast(tickFrame(1, 400));
    await settle();

    expect(tagged).toHaveLength(1);
    expect(tagged[0].authSlot).toBe(1);
  });

  it('drops a forged frame at the star centre — not to the sim, not to a sibling', async () => {
    // A member frame whose declared origin disagrees with its connection is a
    // forgery. The lead is the one host that can catch it — against the connection
    // it arrived on — so it drops it before delivering it to its own sim OR relaying
    // it to a sibling, which is what makes the boundary real for the siblings.
    const delivered = [];
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories, {
      authenticateFrame: () => false, // every frame reads as forged
      onSimulationFrame: (raw) => delivered.push(raw),
    });
    const two = await memberOn(world, factories, lead.code.suffix);
    const siblingFrames = [];
    const three = await memberOn(world, factories, lead.code.suffix, {
      onSimulationFrame: (raw) => siblingFrames.push(raw),
    });
    expect(three.member.slot).toBe('slot-3');

    two.member.broadcast(tickFrame(2, 412));
    await settle();

    expect(delivered).toEqual([]);
    expect(siblingFrames).toEqual([]);
  });

  it('lets a replacement reclaim a disconnected slot, and the owner announces it', async () => {
    // The whole recovery path from the lobby's side: a member drops mid-mission,
    // its slot is kept disconnected, and a replacement typing the code with a claim
    // is seated AS that slot — keeping its frozen ship — while the owner broadcasts
    // the grant the simulation recovers from.
    const claimed = [];
    const { world, factories, lead } = await fleetOf({
      onSlotClaimed: (slot) => claimed.push(slot),
    });
    const two = await memberOn(world, factories, lead.code.suffix, {
      ship: { template_path: 'cruiser.toml' },
    });
    expect(two.member.slot).toBe('slot-2');
    lead.fleet.freeze();
    await settle();
    two.member.close();
    await settle();
    expect(lastRoster(lead).slots.find((s) => s.id === 'slot-2')).toMatchObject({
      connected: false,
    });

    // The replacement machine claims slot 2.
    const replacement = await memberOn(world, factories, lead.code.suffix, {
      claim: 'slot-2',
    });
    await settle();

    // AC1/AC4: seated as slot 2, its frozen ship preserved (no loadout change), and
    // the slot is connected again — rebound to the replacement.
    expect(replacement.member.slot).toBe('slot-2');
    expect(replacement.refusals).toEqual([]);
    const slotTwo = lastRoster(lead).slots.find((s) => s.id === 'slot-2');
    expect(slotTwo).toMatchObject({ connected: true, ship: { template_path: 'cruiser.toml' } });
    // AC2: the owner announced the grant for the simulation to recover from.
    expect(claimed).toEqual([2]);
  });

  it('refuses a replacement that claims a still-connected slot', async () => {
    // AC1: cannot displace a connected host. The claim is refused and the
    // replacement is never seated.
    const { world, factories, lead } = await fleetOf();
    const two = await memberOn(world, factories, lead.code.suffix);
    expect(two.member.slot).toBe('slot-2');
    lead.fleet.freeze();
    await settle();

    const replacement = await memberOn(world, factories, lead.code.suffix, {
      claim: 'slot-2',
    });
    await settle();

    expect(replacement.member.slot).toBeNull();
    expect(replacement.refusals.map((r) => r.reason)).toContain('slot-taken');
    // The live slot 2 is untouched.
    expect(lastRoster(lead).slots.find((s) => s.id === 'slot-2')).toMatchObject({
      connected: true,
    });
  });
});

describe('unbound-connection ingress (issue #1120 adversarial)', () => {
  // Every OTHER test in this file drives a connection that completes
  // hello/welcome and is therefore BOUND — `connSlots` holds its slot. The hole
  // an adversarial review found is the connection that clears the transport
  // compat handshake (build stamp only) but never sends `hello`: `authSlot` is
  // `undefined`, and a "trust what I cannot judge" guard used to let ITS
  // simulation frames through — handed to the lead's own sim as slot 0
  // (MeshOrigin::Unauthenticated → trusted) AND relayed verbatim to every member,
  // who accept a lead relay wholesale. So any machine with the fleet code and a
  // compatible build could inject forged frames under any slot's identity,
  // fleet-wide. These tests drive that exact production path — `authSlot == null`
  // — which the bound-connection tests all miss.

  const tickFrame = (from, tick) =>
    encodeHostFrame(simulationFrame(HOST_FRAME_TICK, {
      from,
      tick,
      ready_through: tick + 2,
      commands: [],
    }, tick));

  /**
   * A ship-host connection that completes the transport compat handshake but is
   * told, per test, what (if anything) to send in its OWN vocabulary — so a test
   * can leave it UNBOUND (never `hello`) or bind it (send `hello`). It is the raw
   * joiner `createFleetMember` wraps, with the hello suppressed.
   */
  function rawServerJoiner(world, factories, code, { onAccepted } = {}) {
    const frames = [];
    const errors = [];
    let joiner = null;
    joiner = createRendezvousJoiner({
      base: 'https://rendezvous.test',
      data: DATA,
      code,
      namespace: NAMESPACE_SERVER,
      stamp: STAMP,
      factories,
      localise: false,
      onAccepted: () => { if (onAccepted) onAccepted(joiner); },
      onData: (frame) => frames.push(frame),
      onError: (reason, detail) => errors.push({ reason, detail }),
    });
    return { joiner, frames, errors };
  }

  it('refuses a simulation frame from an unbound connection — not to the sim, not to a sibling', async () => {
    const delivered = [];
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories, {
      onSimulationFrame: (raw, authSlot) => delivered.push({ raw, authSlot }),
    });
    // A BOUND sibling that any errant relay would reach.
    const sibling = [];
    const bound = await memberOn(world, factories, lead.code.suffix, {
      onSimulationFrame: (raw) => sibling.push(raw),
    });
    expect(bound.member.slot).toBe('slot-2');

    // The attacker: compat-passed, never `hello`'d → no `connSlots` entry.
    const attacker = rawServerJoiner(world, factories, lead.code.suffix);
    await settle();
    // Exploit 1: a forged Tick under slot 2's identity.
    attacker.joiner.sendFrame(tickFrame(2, 412));
    await settle();

    expect(delivered).toEqual([]); // the lead's own sim never saw it
    expect(sibling).toEqual([]); // and it was never relayed to a member
  });

  it('still binds an unbound connection that sends hello, and its bound frame then flows', async () => {
    // The control path is INTACT: a `hello` from an unbound connection is how a
    // connection gets bound in the first place, so it must still be processed —
    // only SIMULATION frames require a prior binding. And once bound, the
    // connection's own simulation frame flows, tagged with its authenticated slot.
    const delivered = [];
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories, {
      onSimulationFrame: (raw, authSlot) => delivered.push({ raw, authSlot }),
    });

    const joined = rawServerJoiner(world, factories, lead.code.suffix, {
      onAccepted: (j) => j.sendFrame(encodeHostFrame(helloFrame({ name: 'Two' }))),
    });
    await settle();

    // The control frame bound it: the roster grew and a welcome came back.
    expect(lastRoster(lead).slots.map((s) => s.id)).toEqual(['slot-1', 'slot-2']);
    const welcome = joined.frames.find((f) => f.t === 'welcome');
    expect(welcome && welcome.d.slot).toBe('slot-2');

    // Now BOUND, its legitimate simulation frame is delivered, tagged slot 2.
    joined.joiner.sendFrame(tickFrame(2, 500));
    await settle();
    expect(delivered).toHaveLength(1);
    expect(delivered[0].authSlot).toBe(2);
  });

  it('refuses an unbound tick-0 host-loss for a LIVE slot — no divergent loss (amplification 3)', async () => {
    // Forged HostLoss{from: victim, lost: victim, tick: 0} on an unbound
    // connection. Delivered to the owner's own sim it would arrive as slot 0
    // (MeshOrigin::Unauthenticated), whose tick-0 self-observation guard
    // (src/lockstep/mod.rs) fires only for MeshOrigin::Peer(_) — so the owner
    // would derive a loss for a LIVE slot while members (tagged Peer(lead)) drop
    // it, diverging. That divergence is reachable ONLY through this JS bridge
    // (Rust deliberately keeps Unauthenticated tick-0 for native fixtures), so it
    // is closed here: the unbound frame never crosses the wasm boundary or gets
    // relayed.
    const delivered = [];
    const world = makeWorld();
    const factories = { socket: world.socket, peer: makePeerFactory() };
    const lead = await leadOn(world, factories, {
      onSimulationFrame: (raw, authSlot) => delivered.push({ raw, authSlot }),
    });
    const victim = [];
    const bound = await memberOn(world, factories, lead.code.suffix, {
      onSimulationFrame: (raw) => victim.push(raw),
    });
    expect(bound.member.slot).toBe('slot-2'); // a LIVE slot

    const attacker = rawServerJoiner(world, factories, lead.code.suffix);
    await settle();
    attacker.joiner.sendFrame(encodeHostFrame(simulationFrame(HOST_FRAME_HOST_LOSS, {
      from: 2,
      lost: 2,
      tick: 0,
    }, 0)));
    await settle();

    expect(delivered).toEqual([]); // never crosses the wasm boundary
    expect(victim).toEqual([]); // never relayed to the live slot's sibling
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
