// The host-to-host vocabulary and the fleet-lobby model (issue #1114).
//
// Two things are under test and they are deliberately in one file, because the
// interesting failures are where they meet: an envelope that a crew decoder
// could mistake for a ClientMessage, and a fleet model that answers a `hello`
// with the wrong one of four possible refusals.

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  HOST_MESH_PROTOCOL,
  HOST_FRAME_TYPES,
  HOST_SIMULATION_FRAME_TYPES,
  HOST_FRAME_TICK,
  HOST_FRAME_DIGEST,
  HOST_FRAME_SNAPSHOT,
  HOST_FRAME_HOST_LOSS,
  HOST_FRAME_SLOT_CLAIM,
  HOST_FRAME_START_STATE,
  HOST_FRAME_START_POLICY,
  HOST_FRAME_START_FORCE,
  HOST_FRAME_FORCE_RESULT,
  isSimulationFrame,
  simulationFrame,
  hostSlotOrdinal,
  ADMISSION_CLOSED,
  ADMISSION_OPEN,
  HOST_ROLE_GM,
  HOST_ROLE_SHIP,
  REASON_ADMISSION_CLOSED,
  REASON_FLEET_FULL,
  REASON_RECOVERY_ONLY,
  REASON_SLOT_TAKEN,
  claimSlot,
  hostFrame,
  encodeHostFrame,
  decodeHostFrame,
  isHostFrame,
  openFleet,
  admitHost,
  decideFirstTimeGmJoin,
  commitFirstTimeGmJoin,
  setAdmission,
  freezeFleet,
  updateSlot,
  setCrewReadiness,
  setGmReady,
  setStartValidation,
  startPolicyOf,
  grantFleetStart,
  adjudicateForceStart,
  dropHost,
  rosterOf,
  simulationRosterOf,
  slotForPeer,
  helloFrame,
  welcomeFrame,
  refusedFrame,
  slotFrame,
  rosterFrame,
  admissionFrame,
  startStateFrame,
  startPolicyFrame,
  startForceFrame,
  forceResultFrame,
  gmJoinRequestFrame,
  gmJoinDecisionFrame,
  gmJoinStatusFrame,
  gmJoinPendingFrame,
  fleetPanelViewModel,
} from '../../gui/host-mesh.js';
import {
  SURFACE_CLIENT,
  SURFACE_SERVER,
  reasonStringId,
  serverSurfaceReasons,
} from '../../gui/join-code.js';
import { buildTable } from '../../gui/strings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const JOIN_DATA = JSON.parse(readFileSync(path.join(root, 'assets/join/join-codes.json'), 'utf8'));
const STRINGS = buildTable(readFileSync(path.join(root, 'assets/strings/strings.csv'), 'utf8'));

const MAX = JOIN_DATA.limits.max_fleet_hosts;
const fleetOf = (over = {}) => openFleet({ maxSlots: MAX, ...over });

/** Seat `n` member hosts, returning the final fleet. */
function withMembers(fleet, n) {
  let f = fleet;
  for (let i = 0; i < n; i += 1) {
    const r = admitHost(f, { peer: `peer-${i}`, name: `Host ${i}` });
    expect(r.ok).toBe(true);
    f = r.fleet;
  }
  return f;
}

describe('the envelope', () => {
  it('round-trips every frame this revision speaks', () => {
    const joinRequest = {
      id: 7,
      candidate: {
        id: 'gm-2', meshSlot: 'slot-3', name: 'Guest GM', credential: 'private-capability',
      },
    };
    const frames = [
      helloFrame({ ship: { template_path: 'a.toml' }, name: 'Two' }),
      welcomeFrame('slot-2', rosterOf(fleetOf())),
      refusedFrame('recovery-only', { detail: 'the mission has started' }),
      slotFrame({ ship: { template_path: 'b.toml' }, ready: true }),
      rosterFrame(fleetOf()),
      admissionFrame(ADMISSION_CLOSED),
      startStateFrame({ crew: { connected: 2, ready: 1 }, validation: true }),
      startPolicyFrame({ connected_total: 2, ready_total: 1 }),
      startForceFrame(),
      forceResultFrame({
        status: 'refused', operator_id: 'gm-1', reason: 'validation-failed', grant_id: null,
      }),
      gmJoinRequestFrame(joinRequest),
      gmJoinDecisionFrame(7, true),
      gmJoinStatusFrame(joinRequest, 'accepted'),
      gmJoinPendingFrame(joinRequest, rosterOf(fleetOf())),
    ];
    for (const frame of frames) {
      expect(HOST_FRAME_TYPES).toContain(frame.t);
      expect(decodeHostFrame(encodeHostFrame(frame))).toEqual(frame);
    }
    // Every declared LOBBY type is exercised above, so one added without a
    // round-trip here fails rather than shipping untested. The simulation's
    // frames are covered by their own case below — this module never builds their
    // bodies, so it cannot round-trip them from a builder it does not have.
    expect(new Set(frames.map((f) => f.t)))
      .toEqual(new Set(HOST_FRAME_TYPES.filter((t) => !HOST_SIMULATION_FRAME_TYPES.includes(t))));
  });

  it('carries the simulation\'s frames without reading them', () => {
    // Issue #1116. A `tick` body is minted by `core::codec::encode_mesh_frame`
    // and read by `decode_mesh_frame`; this module owns the envelope around it
    // and nothing else. So what is under test is the ferrying: the envelope
    // survives, the body arrives byte-identical, and the tick stamp #1114 put
    // in the envelope and never set is finally set.
    const body = {
      from: 2,
      tick: 412,
      ready_through: 418,
      commands: [{
        tick: 418,
        origin: 2,
        seq: 7,
        ship: '00000000-0000-8000-8000-000000000001',
        target: 'helm-steering',
        payload: { SetSteering: { value: -0.4 } },
      }],
    };
    const frame = simulationFrame(HOST_FRAME_TICK, body, 412);
    expect(frame.tick).toBe(412);
    expect(decodeHostFrame(encodeHostFrame(frame))).toEqual(frame);
    expect(decodeHostFrame(encodeHostFrame(frame)).d).toEqual(body);

    const digest = simulationFrame(HOST_FRAME_DIGEST, {
      from: 1,
      tick: 300,
      // A hex STRING, because a digest is a u64 and this is JavaScript: as a
      // number it would round, and the fleet would compare a value neither host
      // actually folded. `core::codec` encodes it this way for that reason.
      digest: 'deadbeefdeadbeef',
    }, 300);
    const back = decodeHostFrame(encodeHostFrame(digest));
    expect(back.d.digest).toBe('deadbeefdeadbeef');
    expect(Number.isSafeInteger(parseInt(back.d.digest, 16))).toBe(false);

    // Issue #1117. A snapshot chunk's body is likewise minted by
    // `core::codec::encode_mesh_frame` and read by `decode_mesh_frame`; this
    // module ferries it unread. Its `whole_hash` and `transfer_id` are u64s and
    // cross as hex strings for the same reason the digest does.
    const chunk = simulationFrame(HOST_FRAME_SNAPSHOT, {
      from: 2,
      transfer_id: '0123456789abcdef',
      tick: 400,
      seq: 3,
      total: 9,
      whole_hash: 'feedfacedeadbeef',
      crc: 3735928559,
      text: 'a RON slice with "quotes"',
    }, 400);
    const chunkBack = decodeHostFrame(encodeHostFrame(chunk));
    expect(chunkBack).toEqual(chunk);
    expect(Number.isSafeInteger(parseInt(chunkBack.d.whole_hash, 16))).toBe(false);

    // The host-loss frame (issue #1119) rides the same envelope: minted by
    // `core::codec::encode_mesh_frame`, ferried here and relayed to siblings
    // without this module reading it. Its body is the lost slot and the agreed
    // disconnect tick.
    const hostLoss = simulationFrame(HOST_FRAME_HOST_LOSS, { from: 1, lost: 3, tick: 418 }, 418);
    expect(decodeHostFrame(encodeHostFrame(hostLoss)).d).toEqual({ from: 1, lost: 3, tick: 418 });
  });

  it('tells the simulation\'s frames from the lobby\'s, which is the routing rule', () => {
    // `gui/fleet-session.js` sends one to the wasm boundary and the other to
    // the fleet model. A lobby frame reaching the simulation would be a roster
    // edit nobody admitted; a tick frame reaching the fleet model would be
    // silently dropped and the fleet would stall on the peer that sent it.
    expect(isSimulationFrame(simulationFrame(HOST_FRAME_TICK, {}))).toBe(true);
    expect(isSimulationFrame(simulationFrame(HOST_FRAME_DIGEST, {}))).toBe(true);
    expect(isSimulationFrame(simulationFrame(HOST_FRAME_SNAPSHOT, {}))).toBe(true);
    expect(isSimulationFrame(simulationFrame(HOST_FRAME_HOST_LOSS, {}))).toBe(true);
    expect(isSimulationFrame(simulationFrame(HOST_FRAME_SLOT_CLAIM, {}))).toBe(true);
    for (const frame of [rosterFrame(fleetOf()), admissionFrame(ADMISSION_CLOSED),
      helloFrame({}), refusedFrame('fleet-full')]) {
      expect(isSimulationFrame(frame), frame.t).toBe(false);
    }
    expect(isSimulationFrame(null)).toBe(false);
  });

  it('reduces a slot id to the ordinal the simulation routes on', () => {
    // A host-loss report (issue #1119) crosses the wasm boundary as the ordinal
    // `N` in `slot-N` — the number `command_admission::HostSlot` orders on. A
    // malformed id is `null`, not a guess, mirroring `HostSlot::from_slot_id`.
    expect(hostSlotOrdinal('slot-3')).toBe(3);
    expect(hostSlotOrdinal('slot-1')).toBe(1);
    expect(hostSlotOrdinal('slot-x')).toBeNull();
    expect(hostSlotOrdinal('3')).toBeNull();
    expect(hostSlotOrdinal(null)).toBeNull();
  });

  it('speaks the same revision as the Rust half', () => {
    // Declared in both halves and pinned in both. A host whose JS speaks one
    // revision and whose Rust speaks another assembles a fleet and then
    // silently fails to agree a tick; refusing an unrecognised `m` is what
    // makes that fail loudly, and this pair of pins is what catches a
    // one-sided bump.
    expect(HOST_MESH_PROTOCOL).toBe(11);
    const rust = readFileSync(
      path.join(path.dirname(fileURLToPath(import.meta.url)), '../../src/lockstep/frame.rs'),
      'utf8',
    );
    expect(rust).toMatch(
      new RegExp(`pub const HOST_MESH_PROTOCOL: u32 = ${HOST_MESH_PROTOCOL};`),
    );
  });

  it('says WHICH request a refusal answers, so a member can tell them apart', () => {
    // The owner refuses from two places — a hello it will not seat, and a slot
    // patch from a host it already seated — and only the first is terminal.
    // Without the subject a member has to treat both as terminal, which throws
    // a legitimate host out of its own fleet for touching its loadout.
    expect(refusedFrame('recovery-only', { of: 'slot' }).d.of).toBe('slot');
    expect(refusedFrame('fleet-full', { of: 'hello' }).d.of).toBe('hello');
    // Unqualified means the admission itself: the older, terminal answer, so a
    // caller that forgets the subject cannot accidentally make a refusal look
    // survivable.
    expect(refusedFrame('fleet-full').d.of).toBe('hello');
  });

  it('carries no delivery stamp in a hello — that check is the transport plane\'s', () => {
    // The authoritative verdict is delivery::check_host_stamp's, made over the
    // in-band JoinHandshake before a hello is ever read. A copy of it here
    // would be a peer-supplied, unvalidated fact sitting in the frame a future
    // reader reaches for first.
    const hello = helloFrame({ ship: { template_path: 'a.toml' }, name: 'Two' });
    expect(hello.d).not.toHaveProperty('stamp');
    expect(Object.keys(hello.d).sort()).toEqual(['name', 'ship']);
  });

  it('carries a tick field from the first frame, unset until #1116 stamps one', () => {
    expect(hostFrame('roster', {}).tick).toBeNull();
    const stamped = hostFrame('roster', {}, { tick: 4200 });
    expect(decodeHostFrame(encodeHostFrame(stamped)).tick).toBe(4200);
  });

  it('cannot be mistaken for a crew message, in either direction', () => {
    // A host frame has no `type`, which is the only key every ClientMessage /
    // ServerMessage decoder in the project switches on.
    const frame = rosterFrame(fleetOf());
    expect(frame).not.toHaveProperty('type');
    expect(frame).not.toHaveProperty('data');
    // And a crew message is not a host frame.
    expect(decodeHostFrame(JSON.stringify({ type: 'Identify', data: { token: 'x' } }))).toBeNull();
    expect(isHostFrame({ type: 'Identify', data: {} })).toBe(false);
  });

  it('refuses a frame from another revision rather than reading its fields', () => {
    const future = { ...rosterFrame(fleetOf()), m: HOST_MESH_PROTOCOL + 1 };
    expect(decodeHostFrame(JSON.stringify(future))).toBeNull();
  });

  it('refuses an unknown type, unparseable text and a non-string payload', () => {
    expect(decodeHostFrame(JSON.stringify({ m: HOST_MESH_PROTOCOL, t: 'launch' }))).toBeNull();
    expect(decodeHostFrame('not json')).toBeNull();
    expect(decodeHostFrame(new Uint8Array([1, 2]))).toBeNull();
    expect(decodeHostFrame(null)).toBeNull();
  });

  it('answers a missing body with an empty one rather than undefined', () => {
    const decoded = decodeHostFrame(JSON.stringify({ m: HOST_MESH_PROTOCOL, t: 'roster' }));
    expect(decoded.d).toEqual({});
  });
});

describe('opening a fleet', () => {
  it('seats the owner in the first slot and admits nobody else yet', () => {
    const fleet = fleetOf({ name: 'Lead', ship: { template_path: 'destroyer.toml' } });
    expect(fleet.slots).toHaveLength(1);
    expect(fleet.slots[0]).toMatchObject({ id: 'slot-1', owner: true, connected: true });
    expect(fleet.owner).toBe('slot-1');
    expect(fleet.admission).toBe(ADMISSION_OPEN);
    expect(fleet.frozen).toBe(false);
  });

  it('takes its capacity from the authored table rather than inventing one', () => {
    expect(fleetOf().maxSlots).toBe(MAX);
    expect(Number.isInteger(MAX)).toBe(true);
    // The service's own per-record bound must be the looser of the two, or the
    // fleet's designed size would be silently clipped by a memory limit.
    expect(MAX).toBeLessThanOrEqual(JOIN_DATA.limits.max_peers_per_record);
  });
});

describe('privileged GM host role (issue #1289)', () => {
  const credentials = (...values) => {
    let index = 0;
    return () => values[index++];
  };

  it('keeps an absent role as the existing ship-host behaviour', () => {
    const fleet = fleetOf({ name: 'Lead', ship: { template_path: 'destroyer.toml' } });
    expect(fleet.slots).toHaveLength(1);
    expect(fleet.gms).toEqual([]);
    expect(helloFrame({ name: 'Two' }).d).toEqual({ ship: null, name: 'Two' });
    expect(admitHost(fleet, { peer: 'p2' })).toMatchObject({
      ok: true,
      role: HOST_ROLE_SHIP,
      meshSlot: 'slot-2',
    });
  });

  it('represents a GM-only owner without inventing a ship or public GM leader', () => {
    const fleet = fleetOf({
      role: HOST_ROLE_GM,
      name: 'Morgan',
      ship: { template_path: 'must-not-survive.toml' },
      credentialFactory: credentials('owner-private-capability'),
    });
    const roster = rosterOf(fleet);

    // The technical owner is still the deterministic star centre, but it is
    // not a ship row and does not confer public leadership on the GM row.
    expect(fleet.owner).toBe('slot-1');
    expect(fleet.slots).toEqual([]);
    expect(roster.gms).toEqual([{ id: 'gm-1', name: 'Morgan', connected: true, ready: false }]);
    expect(Object.keys(roster.gms[0]).sort()).toEqual(['connected', 'id', 'name', 'ready']);
    expect(roster.gms[0]).not.toHaveProperty('owner');
    expect(roster.gms[0]).not.toHaveProperty('permissions');
    expect(JSON.stringify(roster)).not.toContain('owner-private-capability');
  });

  it('admits equal GM operators without consuming authored ship capacity', () => {
    let fleet = fleetOf({ credentialFactory: credentials('gm-a-secret', 'gm-b-secret') });
    const first = admitHost(fleet, { peer: 'gm-peer-a', role: HOST_ROLE_GM, name: 'A' });
    fleet = first.fleet;
    const second = admitHost(fleet, { peer: 'gm-peer-b', role: HOST_ROLE_GM, name: 'B' });
    fleet = second.fleet;

    expect(first).toMatchObject({
      role: HOST_ROLE_GM,
      meshSlot: 'slot-2',
      operatorId: 'gm-1',
      reconnectCredential: 'gm-a-secret',
    });
    expect(second).toMatchObject({
      role: HOST_ROLE_GM,
      meshSlot: 'slot-3',
      operatorId: 'gm-2',
      reconnectCredential: 'gm-b-secret',
    });
    expect(fleet.slots).toHaveLength(1);
    expect(rosterOf(fleet).gms).toEqual([
      { id: 'gm-1', name: 'A', connected: true, ready: false },
      { id: 'gm-2', name: 'B', connected: true, ready: false },
    ]);

    // GM peers are extra host-class simulations, not player-ship rows. Every
    // authored ship slot remains available after both have joined.
    fleet = withMembers(fleet, MAX - 1);
    expect(fleet.slots).toHaveLength(MAX);
    expect(fleet.gms).toHaveLength(2);
  });

  it('rebinds a disconnected GM by private credential even after admission closes', () => {
    const admitted = admitHost(
      fleetOf({ credentialFactory: credentials('reconnect-me') }),
      { peer: 'old-peer', role: HOST_ROLE_GM, name: 'Morgan' },
    );
    const disconnected = dropHost(admitted.fleet, 'old-peer');
    const closed = setAdmission(disconnected, ADMISSION_CLOSED);
    const rebound = admitHost(closed, {
      peer: 'new-peer',
      role: HOST_ROLE_GM,
      reconnectCredential: admitted.reconnectCredential,
      name: 'an attacker cannot rename on reconnect',
    });

    expect(rebound).toMatchObject({
      ok: true,
      reconnected: true,
      operatorId: 'gm-1',
      meshSlot: 'slot-2',
      reconnectCredential: 'reconnect-me',
    });
    expect(rosterOf(rebound.fleet).gms).toEqual([
      { id: 'gm-1', name: 'Morgan', connected: true, ready: false },
    ]);
    expect(rebound.fleet.gms).toHaveLength(1);
  });

  it('auto-pends a known frozen GM without changing its public row or identity counters', () => {
    const admitted = admitHost(
      fleetOf({ credentialFactory: credentials('only-real-secret') }),
      { peer: 'old-peer', role: HOST_ROLE_GM },
    );
    const frozen = freezeFleet(dropHost(admitted.fleet, 'old-peer'));

    const bad = admitHost(frozen, {
      peer: 'attacker',
      role: HOST_ROLE_GM,
      reconnectCredential: 'wrong-secret',
    });
    expect(bad).toEqual({ ok: false, reason: REASON_RECOVERY_ONLY });
    expect(frozen.gms).toHaveLength(1);
    expect(frozen.gms[0]).toMatchObject({ connected: false, peer: null });

    const rebound = admitHost(frozen, {
      peer: 'replacement',
      role: HOST_ROLE_GM,
      reconnectCredential: 'only-real-secret',
    });
    expect(rebound).toMatchObject({
      ok: true,
      pending: true,
      reconnect: true,
      operatorId: 'gm-1',
      meshSlot: 'slot-2',
      reconnectCredential: 'only-real-secret',
      request: { kind: 'reconnect', status: 'accepted' },
    });
    expect(rebound.fleet.nextSeq).toBe(frozen.nextSeq);
    expect(rebound.fleet.nextGmSeq).toBe(frozen.nextGmSeq);
    expect(rosterOf(rebound.fleet).gms).toEqual([
      { id: 'gm-1', name: '', connected: false, ready: false },
    ]);
    expect(frozen.gms).toHaveLength(1);
    expect(frozen.gms[0]).toMatchObject({ connected: false, peer: null });

    const committed = commitFirstTimeGmJoin(rebound.fleet, rebound.request.id);
    expect(committed).toMatchObject({ ok: true, gm: {
      id: 'gm-1', meshSlot: 'slot-2', peer: 'replacement',
      name: '', credential: 'only-real-secret', connected: true,
    } });
    expect(committed.fleet.gms).toHaveLength(1);
    expect(rosterOf(committed.fleet).gms).toEqual([
      { id: 'gm-1', name: '', connected: true, ready: false },
    ]);
  });

  it('replays the exact frozen reconnect and deterministically refuses a competitor', () => {
    const admitted = admitHost(
      fleetOf({ credentialFactory: credentials('stable-secret') }),
      { peer: 'old-peer', role: HOST_ROLE_GM, name: 'Morgan' },
    );
    const frozen = freezeFleet(dropHost(admitted.fleet, 'old-peer'));
    const first = admitHost(frozen, {
      peer: 'winner', role: HOST_ROLE_GM, reconnectCredential: 'stable-secret',
    });
    const retry = admitHost(first.fleet, {
      peer: 'winner', role: HOST_ROLE_GM, reconnectCredential: 'stable-secret',
    });
    expect(retry).toMatchObject({
      ok: true, pending: true, duplicate: true,
      request: { id: first.request.id, kind: 'reconnect' },
    });
    expect(retry.fleet).toBe(first.fleet);

    const competitor = admitHost(first.fleet, {
      peer: 'competitor', role: HOST_ROLE_GM, reconnectCredential: 'stable-secret',
    });
    expect(competitor).toEqual({ ok: false, reason: 'join-in-progress' });
    expect(rosterOf(first.fleet).gms).toEqual([
      { id: 'gm-1', name: 'Morgan', connected: false, ready: false },
    ]);
  });

  it('keeps a first-time mid-session GM private until matching-digest commit', () => {
    const frozen = freezeFleet(fleetOf({
      credentialFactory: credentials('join-secret-a', 'join-secret-b'),
    }));
    const publicBefore = rosterOf(frozen);

    const rejectedRequest = admitHost(frozen, {
      peer: 'candidate-a', role: HOST_ROLE_GM, name: 'A',
    });
    expect(rejectedRequest).toMatchObject({ ok: true, pending: true, operatorId: 'gm-1' });
    expect(rosterOf(rejectedRequest.fleet)).toEqual(publicBefore);
    const rejected = decideFirstTimeGmJoin(
      rejectedRequest.fleet, rejectedRequest.request.id, false, 'slot-1',
    );
    expect(rejected).toMatchObject({ ok: true, accepted: false });
    expect(rosterOf(rejected.fleet)).toEqual(publicBefore);

    const requested = admitHost(rejected.fleet, {
      peer: 'candidate-b', role: HOST_ROLE_GM, name: 'B',
    });
    const accepted = decideFirstTimeGmJoin(
      requested.fleet, requested.request.id, true, 'slot-1',
    );
    expect(accepted).toMatchObject({ ok: true, accepted: true });
    expect(rosterOf(accepted.fleet)).toEqual(publicBefore);
    expect(accepted.fleet.pendingGmJoin.status).toBe('accepted');

    const committed = commitFirstTimeGmJoin(accepted.fleet, requested.request.id);
    expect(committed.ok).toBe(true);
    expect(rosterOf(committed.fleet).gms).toEqual([
      { id: 'gm-2', name: 'B', connected: true, ready: false },
    ]);
    expect(rosterOf(committed.fleet).participants).toContain('slot-3');
  });

  it('puts role and reconnect capability only in the privileged host envelope', () => {
    const hello = helloFrame({
      role: HOST_ROLE_GM,
      name: 'Morgan',
      ship: { template_path: 'ignored.toml' },
      reconnectCredential: 'private-capability',
    });
    expect(hello.d).toEqual({
      role: HOST_ROLE_GM,
      name: 'Morgan',
      reconnect_credential: 'private-capability',
    });
    expect(hello).not.toHaveProperty('type');
    expect(hello).not.toHaveProperty('data');

    const roster = rosterOf(fleetOf());
    const welcome = welcomeFrame('slot-2', roster, {
      role: HOST_ROLE_GM,
      operatorId: 'gm-1',
      reconnectCredential: 'private-capability',
    });
    expect(welcome.d).toMatchObject({
      role: HOST_ROLE_GM,
      operator_id: 'gm-1',
      reconnect_credential: 'private-capability',
    });
    expect(JSON.stringify(welcome.d.roster)).not.toContain('private-capability');
  });
});

describe('private deterministic participant roster (issue #1290)', () => {
  const credentials = (...values) => {
    let index = 0;
    return () => values[index++];
  };

  it('orders every connected technical peer while keeping GM-slot mapping private', () => {
    let fleet = fleetOf({
      ship: { template_path: 'lead.toml' },
      credentialFactory: credentials('private-gm-capability'),
    });
    fleet = admitHost(fleet, {
      peer: 'gm-secret-peer', role: HOST_ROLE_GM, name: 'Morgan',
    }).fleet;
    fleet = admitHost(fleet, {
      peer: 'ship-three', ship: { template_path: 'three.toml' }, name: 'Three',
    }).fleet;

    const roster = rosterOf(fleet);
    expect(roster.participants).toEqual(['slot-1', 'slot-2', 'slot-3']);
    expect(roster.gms).toEqual([
      { id: 'gm-1', name: 'Morgan', connected: true, ready: false },
    ]);
    expect(roster.gms[0]).not.toHaveProperty('meshSlot');
    expect(JSON.stringify(roster)).not.toContain('gm-secret-peer');
    expect(JSON.stringify(roster)).not.toContain('private-gm-capability');

    expect(simulationRosterOf(roster, 'slot-2')).toEqual({
      local: 2,
      owner: 1,
      participants: [1, 2, 3],
      gms: [{ host: 2, operator_id: 'gm-1' }],
      ships: [
        { host: 1, ship_path: 'lead.toml', crew: [] },
        { host: 3, ship_path: 'three.toml', crew: [] },
      ],
    });
  });

  it('emits a valid zero-ship topology for a GM-only owner', () => {
    const roster = rosterOf(fleetOf({
      role: HOST_ROLE_GM,
      credentialFactory: credentials('owner-secret'),
    }));
    expect(roster.participants).toEqual(['slot-1']);
    expect(simulationRosterOf(roster, 'slot-1')).toEqual({
      local: 1,
      owner: 1,
      participants: [1],
      gms: [{ host: 1, operator_id: 'gm-1' }],
      ships: [],
    });
  });

  it('excludes disconnected technical rows and refuses a local slot outside the set', () => {
    const admitted = admitHost(
      fleetOf({ credentialFactory: credentials('gm-secret') }),
      { peer: 'gm-peer', role: HOST_ROLE_GM },
    );
    const roster = rosterOf(dropHost(admitted.fleet, 'gm-peer'));
    expect(roster.participants).toEqual(['slot-1']);
    expect(simulationRosterOf(roster, 'slot-2')).toBeNull();
  });
});

describe('collective GM and crew start policy (issue #1290)', () => {
  const credentials = (...values) => {
    let index = 0;
    return () => values[index++];
  };

  const apply = (result) => {
    expect(result.ok).toBe(true);
    return result.fleet;
  };

  it('aggregates every ship-local player tally and every equal GM vote separately', () => {
    let fleet = fleetOf({
      ship: { template_path: 'lead.toml' },
      credentialFactory: credentials('gm-a', 'gm-b'),
    });
    fleet = admitHost(fleet, {
      peer: 'ship-two', ship: { template_path: 'two.toml' }, name: 'Two',
    }).fleet;
    fleet = admitHost(fleet, { peer: 'gm-a', role: HOST_ROLE_GM, name: 'A' }).fleet;
    fleet = admitHost(fleet, { peer: 'gm-b', role: HOST_ROLE_GM, name: 'B' }).fleet;

    for (const slot of fleet.slots) {
      fleet = apply(updateSlot(fleet, slot.id, { ready: true }));
      fleet = apply(setStartValidation(fleet, slot.id, true));
    }
    for (const gm of fleet.gms) fleet = apply(setStartValidation(fleet, gm.meshSlot, true));
    fleet = apply(setCrewReadiness(fleet, 'slot-1', {
      connected: 2,
      ready: 2,
      // Rust excluded these before producing the tally; an extra field cannot
      // make the mesh invent participants.
      spectators: 99,
    }));
    fleet = apply(setCrewReadiness(fleet, 'slot-2', { connected: 1, ready: 0 }));
    fleet = apply(setGmReady(fleet, 'gm-1', true));

    expect(startPolicyOf(fleet)).toEqual({
      connected_players: 3,
      ready_players: 2,
      connected_gms: 2,
      ready_gms: 1,
      connected_total: 5,
      ready_total: 3,
      all_ready: false,
      validation_passed: true,
      can_auto_start: false,
      started: false,
    });

    fleet = apply(setCrewReadiness(fleet, 'slot-2', { connected: 1, ready: 1 }));
    fleet = apply(setGmReady(fleet, 'gm-2', true));
    expect(startPolicyOf(fleet)).toMatchObject({
      connected_total: 5,
      ready_total: 5,
      all_ready: true,
      validation_passed: true,
      can_auto_start: true,
    });
  });

  it('starts a GM-only roster by the same rule and clears a disconnected GM vote', () => {
    let fleet = fleetOf({
      role: HOST_ROLE_GM,
      credentialFactory: credentials('owner', 'member'),
    });
    fleet = admitHost(fleet, { peer: 'gm-member', role: HOST_ROLE_GM }).fleet;
    for (const gm of fleet.gms) fleet = apply(setStartValidation(fleet, gm.meshSlot, true));
    fleet = apply(setGmReady(fleet, 'gm-1', true));
    fleet = apply(setGmReady(fleet, 'gm-2', true));
    expect(startPolicyOf(fleet)).toMatchObject({
      connected_players: 0,
      connected_gms: 2,
      all_ready: true,
      can_auto_start: true,
    });

    fleet = dropHost(fleet, 'gm-member');
    expect(rosterOf(fleet).gms[1]).toEqual({
      id: 'gm-2', name: '', connected: false, ready: false,
    });
    expect(startPolicyOf(fleet)).toMatchObject({
      connected_gms: 1,
      ready_gms: 1,
      connected_total: 1,
      ready_total: 1,
      all_ready: true,
      can_auto_start: true,
    });
  });

  it('lets a connected GM bypass readiness only, never validation', () => {
    let fleet = fleetOf({
      role: HOST_ROLE_GM,
      credentialFactory: credentials('owner'),
    });
    const refused = adjudicateForceStart(fleet, 'gm-1');
    expect(refused.result).toEqual({
      status: 'refused',
      operator_id: 'gm-1',
      reason: 'validation-failed',
      grant_id: null,
    });
    expect(refused.fleet.startGrant).toBeNull();

    fleet = apply(setStartValidation(fleet, 'slot-1', true));
    const applied = adjudicateForceStart(fleet, 'gm-1');
    expect(applied.result).toEqual({
      status: 'applied',
      operator_id: 'gm-1',
      reason: null,
      grant_id: 'start-1',
    });
    expect(applied.grant).toEqual({
      id: 'start-1', mode: 'forced', operator_id: 'gm-1',
    });
    expect(applied.fleet.frozen).toBe(true);

    const duplicate = adjudicateForceStart(applied.fleet, 'gm-1');
    expect(duplicate.result).toEqual({
      status: 'no-op',
      operator_id: 'gm-1',
      reason: 'already-started',
      grant_id: 'start-1',
    });
    expect(duplicate.grant).toBe(applied.grant);

    // A ship or invented/disconnected public id is not GM authority.
    expect(adjudicateForceStart(fleetOf(), 'slot-1')).toMatchObject({ ok: false, result: null });
    expect(adjudicateForceStart(dropHost(fleet, null), 'gm-404'))
      .toMatchObject({ ok: false, result: null });
  });

  it('keeps slot.ready as loadout validation, separate from crew readiness', () => {
    let fleet = fleetOf({ ship: { template_path: 'lead.toml' } });
    fleet = apply(setCrewReadiness(fleet, 'slot-1', { connected: 1, ready: 1 }));
    fleet = apply(setStartValidation(fleet, 'slot-1', true));
    expect(startPolicyOf(fleet)).toMatchObject({
      all_ready: true,
      validation_passed: false,
      can_auto_start: false,
    });
    fleet = apply(updateSlot(fleet, 'slot-1', { ready: true }));
    expect(startPolicyOf(fleet)).toMatchObject({
      all_ready: true,
      validation_passed: true,
      can_auto_start: true,
    });
    expect(rosterOf(fleet).slots[0]).toMatchObject({
      ready: true,
      crew: { connected: 1, ready: 1 },
    });
  });

  it('bounds malformed and enormous crew tallies before exact aggregation', () => {
    let fleet = withMembers(fleetOf(), MAX - 1);
    for (const slot of fleet.slots) {
      fleet = apply(setCrewReadiness(fleet, slot.id, {
        connected: Number.MAX_SAFE_INTEGER,
        ready: Number.MAX_SAFE_INTEGER,
      }));
    }
    const policy = startPolicyOf(fleet);
    expect(Number.isSafeInteger(policy.connected_players)).toBe(true);
    expect(policy.connected_players).toBeLessThanOrEqual(0xffff_ffff);
    expect(policy.ready_players).toBe(policy.connected_players);

    fleet = apply(setCrewReadiness(fleet, 'slot-1', {
      connected: '999', ready: Infinity,
    }));
    expect(rosterOf(fleet).slots[0].crew).toEqual({ connected: 0, ready: 0 });
    fleet = apply(setCrewReadiness(fleet, 'slot-1', { connected: 2, ready: 200 }));
    expect(rosterOf(fleet).slots[0].crew).toEqual({ connected: 2, ready: 2 });
  });

  it('makes the first grant the authoritative boundary against a late withdrawal', () => {
    let fleet = fleetOf({ ship: { template_path: 'lead.toml' } });
    fleet = apply(updateSlot(fleet, 'slot-1', { ready: true }));
    fleet = apply(setCrewReadiness(fleet, 'slot-1', { connected: 1, ready: 1 }));
    fleet = apply(setStartValidation(fleet, 'slot-1', true));
    expect(startPolicyOf(fleet).can_auto_start).toBe(true);

    const first = grantFleetStart(fleet, { mode: 'automatic' });
    expect(first.created).toBe(true);
    expect(first.grant).toEqual({ id: 'start-1', mode: 'automatic', operator_id: null });
    // A withdrawal that was locally clicked but arrives after the grant is too
    // late: the frozen owner refuses the state edit and every host keeps the
    // same immutable grant.
    expect(setCrewReadiness(first.fleet, 'slot-1', { connected: 1, ready: 0 }))
      .toEqual({ ok: false, reason: REASON_RECOVERY_ONLY });
    const repeated = grantFleetStart(first.fleet, { mode: 'automatic' });
    expect(repeated.created).toBe(false);
    expect(repeated.grant).toBe(first.grant);
    expect(startPolicyOf(repeated.fleet)).toMatchObject({ started: true, can_auto_start: false });
  });
});

describe('admitting a second host', () => {
  it('mints a separate slot, numbered by the owner', () => {
    const { ok, fleet, slot } = admitHost(fleetOf(), { peer: 'p2', name: 'Two' });
    expect(ok).toBe(true);
    expect(slot.id).toBe('slot-2');
    expect(slot.owner).toBe(false);
    expect(fleet.slots.map((s) => s.id)).toEqual(['slot-1', 'slot-2']);
  });

  it('answers a repeated hello with the slot that host already holds', () => {
    const first = admitHost(fleetOf(), { peer: 'p2' });
    const again = admitHost(first.fleet, { peer: 'p2' });
    expect(again.ok).toBe(true);
    expect(again.slot.id).toBe('slot-2');
    expect(again.fleet.slots).toHaveLength(2);
  });

  it('never reuses a slot number, even after the host that held it left', () => {
    const seated = admitHost(fleetOf(), { peer: 'p2' }).fleet;
    const gone = dropHost(seated, 'p2');
    expect(gone.slots).toHaveLength(1);
    // slot-2 is spent: an id is a function of the order it was minted in, and
    // #1116 will put these ids in a shared command stream.
    expect(admitHost(gone, { peer: 'p3' }).slot.id).toBe('slot-3');
  });

  it('refuses past the authored capacity, with its own reason', () => {
    const full = withMembers(fleetOf(), MAX - 1);
    expect(full.slots).toHaveLength(MAX);
    expect(admitHost(full, { peer: 'one-too-many' }))
      .toEqual({ ok: false, reason: REASON_FLEET_FULL });
  });
});

describe('closing and reopening admission', () => {
  it('refuses a new host while closed and admits again once reopened', () => {
    const open = fleetOf();
    const closed = setAdmission(open, ADMISSION_CLOSED);
    expect(admitHost(closed, { peer: 'p2' }))
      .toEqual({ ok: false, reason: REASON_ADMISSION_CLOSED });
    const reopened = setAdmission(closed, ADMISSION_OPEN);
    expect(admitHost(reopened, { peer: 'p2' }).ok).toBe(true);
  });

  it('does not disturb a host that is already in the fleet', () => {
    const seated = admitHost(fleetOf(), { peer: 'p2' }).fleet;
    const closed = setAdmission(seated, ADMISSION_CLOSED);
    // Same slot, still connected — closing is an answer to future joiners, not
    // an eviction.
    expect(slotForPeer(closed, 'p2')).toMatchObject({ id: 'slot-2', connected: true });
    expect(closed.slots).toHaveLength(2);
    // And that host may still change its own hull: only mission start freezes.
    expect(updateSlot(closed, 'slot-2', { ready: true }).ok).toBe(true);
  });
});

describe('mission start freezes the topology', () => {
  it('refuses a new host with a recovery reason, not a closure one', () => {
    const frozen = freezeFleet(admitHost(fleetOf(), { peer: 'p2' }).fleet);
    expect(admitHost(frozen, { peer: 'p3' }))
      .toEqual({ ok: false, reason: REASON_RECOVERY_ONLY });
    // The distinction is the point: a closed fleet can be reopened, a launched
    // one cannot, and #1120 lands as a change of answer to THIS reason.
    expect(REASON_RECOVERY_ONLY).not.toBe(REASON_ADMISSION_CLOSED);
  });

  it('refuses a loadout change from a host already in the fleet', () => {
    const frozen = freezeFleet(admitHost(fleetOf(), { peer: 'p2' }).fleet);
    expect(updateSlot(frozen, 'slot-2', { ship: { template_path: 'other.toml' } }))
      .toEqual({ ok: false, reason: REASON_RECOVERY_ONLY });
  });

  it('closes admission as well, so the roster never advertises a slot it cannot mint', () => {
    expect(freezeFleet(fleetOf()).admission).toBe(ADMISSION_CLOSED);
    // Reopening admission does not thaw the roster.
    const thawAttempt = setAdmission(freezeFleet(fleetOf()), ADMISSION_OPEN);
    expect(thawAttempt.frozen).toBe(true);
    expect(admitHost(thawAttempt, { peer: 'p2' }).reason).toBe(REASON_RECOVERY_ONLY);
  });

  it('keeps a dropped slot for recovery instead of deleting it', () => {
    const frozen = freezeFleet(admitHost(fleetOf(), { peer: 'p2' }).fleet);
    const dropped = dropHost(frozen, 'p2');
    expect(dropped.slots).toHaveLength(2);
    expect(dropped.slots[1]).toMatchObject({ id: 'slot-2', connected: false, peer: null });
  });

  it('never drops the owner, frozen or not', () => {
    const fleet = fleetOf();
    expect(dropHost(fleet, null).slots).toHaveLength(1);
    expect(dropHost(freezeFleet(fleet), null).slots).toHaveLength(1);
  });
});

describe('claiming a disconnected slot on another machine (issue #1120)', () => {
  // AC1: server-code entry can select ONLY a disconnected fixed slot, and cannot
  // add a ship, change its loadout or displace a connected host.

  /** A frozen two-host fleet whose slot 2 has disconnected. */
  const withDisconnectedSlotTwo = () => {
    const frozen = freezeFleet(
      admitHost(fleetOf(), { peer: 'p2', ship: { template_path: 'cruiser.toml' } }).fleet,
    );
    return dropHost(frozen, 'p2');
  };

  it('reclaims a disconnected slot, KEEPING its frozen ship — no loadout change', () => {
    const fleet = withDisconnectedSlotTwo();
    const result = claimSlot(fleet, { peer: 'p9', slotId: 'slot-2' });
    expect(result.ok).toBe(true);
    expect(result.slot).toMatchObject({
      id: 'slot-2',
      connected: true,
      peer: 'p9',
      // The frozen ship is preserved verbatim: the claim carries no loadout, so a
      // replacement resumes the same ship and cannot change it.
      ship: { template_path: 'cruiser.toml' },
    });
  });

  it('refuses to displace a still-CONNECTED host', () => {
    const fleet = freezeFleet(admitHost(fleetOf(), { peer: 'p2' }).fleet);
    // slot 2 is still connected — a claim on it is refused, never a takeover.
    expect(claimSlot(fleet, { peer: 'p9', slotId: 'slot-2' })).toEqual({
      ok: false,
      reason: REASON_SLOT_TAKEN,
    });
  });

  it('refuses to claim the owner\'s own slot', () => {
    const fleet = withDisconnectedSlotTwo();
    expect(claimSlot(fleet, { peer: 'p9', slotId: 'slot-1' }).reason).toBe(REASON_SLOT_TAKEN);
  });

  it('refuses a claim on a slot that does not exist', () => {
    const fleet = withDisconnectedSlotTwo();
    expect(claimSlot(fleet, { peer: 'p9', slotId: 'slot-7' }).reason).toBe('unknown');
  });

  it('only recovers after the freeze — before it, there is no fixed slot to reclaim', () => {
    const fleet = admitHost(fleetOf(), { peer: 'p2' }).fleet; // not frozen
    expect(claimSlot(fleet, { peer: 'p9', slotId: 'slot-2' }).reason).toBe(REASON_RECOVERY_ONLY);
  });

  it('cannot add a NEW ship — a claim only ever names an existing slot', () => {
    // There is no path by which a claim creates a slot: `claimSlot` looks the id
    // up and refuses `unknown` when it is not already in the frozen roster, so
    // server-code entry can never grow the fleet.
    const fleet = withDisconnectedSlotTwo();
    expect(fleet.slots).toHaveLength(2);
    const after = claimSlot(fleet, { peer: 'p9', slotId: 'slot-2' }).fleet;
    expect(after.slots).toHaveLength(2);
  });
});

describe('updating a slot', () => {
  it('writes only the fields a member owns about itself', () => {
    const seated = admitHost(fleetOf(), { peer: 'p2', name: 'Two' }).fleet;
    const { ok, fleet } = updateSlot(seated, 'slot-2', {
      ship: { template_path: 'cruiser.toml' },
      ready: true,
      id: 'slot-1',
      owner: true,
    });
    expect(ok).toBe(true);
    expect(fleet.slots[1]).toMatchObject({
      id: 'slot-2',
      owner: false,
      ready: true,
      ship: { template_path: 'cruiser.toml' },
    });
  });

  it('refuses a slot nobody holds', () => {
    expect(updateSlot(fleetOf(), 'slot-9', { ready: true }).ok).toBe(false);
  });
});

describe('what a member may say about itself', () => {
  // `name` and `ship` are the only two fields one host writes into every other
  // host's roster, and the lead re-encodes that roster to the whole fleet on
  // every change. So they get a SHAPE on the way in, the same discipline the
  // rendezvous registry applies to the one host-supplied value it indexes on.
  const NAME_CAP = JOIN_DATA.limits.max_slot_name_length;
  const PATH_CAP = JOIN_DATA.limits.max_slot_ship_path_length;
  /** A fleet carrying the AUTHORED bounds, as server.html builds one. */
  const boundedFleetOf = (over = {}) => openFleet({
    maxSlots: MAX,
    maxNameLength: NAME_CAP,
    maxShipPathLength: PATH_CAP,
    ...over,
  });

  it('bounds a name at the authored length, at the door and on every update', () => {
    const long = 'N'.repeat(NAME_CAP + 500);
    const seated = admitHost(boundedFleetOf(), { peer: 'p2', name: long }).fleet;
    expect(seated.slots[1].name).toHaveLength(NAME_CAP);
    const patched = updateSlot(seated, 'slot-2', { name: long }).fleet;
    expect(patched.slots[1].name).toHaveLength(NAME_CAP);
  });

  it('falls back to a bound of its own when a caller passes none', () => {
    // `openFleet` is called with the authored table in the product, and the
    // parse-time default is what an older table loads under (AGENTS.md rule
    // 11a) — never an absence of a bound.
    const seated = admitHost(fleetOf(), { peer: 'p2', name: 'N'.repeat(5_000) }).fleet;
    expect(seated.slots[1].name.length).toBeLessThanOrEqual(NAME_CAP);
    expect(seated.slots[1].name.length).toBeGreaterThan(0);
  });

  it('bounds the hull path and keeps only the two fields a roster carries', () => {
    const seated = admitHost(boundedFleetOf(), {
      peer: 'p2',
      ship: {
        template_path: `a${'/deep'.repeat(400)}.toml`,
        name: 'X'.repeat(NAME_CAP + 50),
        // Nothing else survives: a roster row is a hull and a label, and an
        // extra field here would be one member's payload on every viewscreen.
        payload: 'Z'.repeat(100_000),
        nested: { and: ['more'] },
      },
    }).fleet;
    const ship = seated.slots[1].ship;
    expect(Object.keys(ship).sort()).toEqual(['name', 'template_path']);
    expect(ship.template_path).toHaveLength(PATH_CAP);
    expect(ship.name).toHaveLength(NAME_CAP);
  });

  it('reads anything that is not a usable ship as "no hull chosen yet"', () => {
    for (const bogus of [42, 'destroyer.toml', [], { name: 'no path' }, { template_path: '' }]) {
      const seated = admitHost(boundedFleetOf(), { peer: 'p2', ship: bogus }).fleet;
      expect(seated.slots[1].ship, JSON.stringify(bogus)).toBeNull();
    }
    // …and null stays a real answer rather than becoming a shape.
    expect(admitHost(boundedFleetOf(), { peer: 'p2' }).fleet.slots[1].ship).toBeNull();
  });

  it('holds the owner\'s own slot to the same rule', () => {
    const fleet = boundedFleetOf({
      name: 'L'.repeat(NAME_CAP + 9),
      ship: { template_path: 'a.toml', junk: 1 },
    });
    expect(fleet.slots[0].name).toHaveLength(NAME_CAP);
    expect(fleet.slots[0].ship).toEqual({ template_path: 'a.toml' });
  });
});

describe('the roster that crosses the wire', () => {
  it('carries the lobby and not the transport', () => {
    const roster = rosterOf(admitHost(fleetOf({ name: 'Lead' }), { peer: 'secret-peer-id' }).fleet);
    expect(JSON.stringify(roster)).not.toContain('secret-peer-id');
    for (const slot of roster.slots) expect(slot).not.toHaveProperty('peer');
    expect(roster).toMatchObject({ owner: 'slot-1', admission: ADMISSION_OPEN, frozen: false });
    expect(roster.slots.map((s) => s.id)).toEqual(['slot-1', 'slot-2']);
  });
});

describe('the fleet panel view model', () => {
  it('is invisible until a fleet exists', () => {
    expect(fleetPanelViewModel(null).visible).toBe(false);
  });

  it('marks this host\'s own row and the lead\'s', () => {
    const roster = rosterOf(admitHost(fleetOf(), { peer: 'p2' }).fleet);
    const vm = fleetPanelViewModel(roster, { suffix: 'QUARK', mine: 'slot-2' });
    expect(vm.code).toBe('QUARK');
    expect(vm.rows.map((r) => [r.owner, r.mine])).toEqual([[true, false], [false, true]]);
    expect(vm.rows[0].label).toEqual({ id: 'server.fleet.slot_owner', params: { n: '1' } });
    expect(vm.rows[1].label).toEqual({ id: 'server.fleet.slot_member', params: { n: '2' } });
  });

  it('says which of the three fleet states it is in, and only one', () => {
    const open = rosterOf(fleetOf());
    expect(fleetPanelViewModel(open).status)
      .toEqual({ id: 'server.fleet.open', params: { n: '1', max: String(MAX) } });
    expect(fleetPanelViewModel(rosterOf(setAdmission(fleetOf(), ADMISSION_CLOSED))).status.id)
      .toBe('server.fleet.closed');
    // Frozen wins over closed, because freezing also closes.
    expect(fleetPanelViewModel(rosterOf(freezeFleet(fleetOf()))).status.id)
      .toBe('server.fleet.frozen');
  });

  it('reports a slot with no hull yet rather than rendering it blank', () => {
    const vm = fleetPanelViewModel(rosterOf(fleetOf()));
    expect(vm.rows[0].ship).toBeNull();
  });

  it('prefers a ship\'s display name over its template path', () => {
    const named = fleetOf({ ship: { template_path: 'a/b/destroyer.toml', name: 'Ironveil' } });
    expect(fleetPanelViewModel(rosterOf(named)).rows[0].ship).toBe('Ironveil');
    const unnamed = fleetOf({ ship: { template_path: 'a/b/destroyer.toml' } });
    expect(fleetPanelViewModel(rosterOf(unnamed)).rows[0].ship).toBe('a/b/destroyer.toml');
  });
});

describe('every reason this module can produce has a sentence', () => {
  // The same discipline gui/join-code.js already keeps for the rendezvous
  // service's reasons and the host's StampMismatch codes: a refusal with no row
  // renders as the misleading "unknown" fallback.
  it.each([REASON_ADMISSION_CLOSED, REASON_FLEET_FULL, REASON_RECOVERY_ONLY])(
    'maps %s to a string id that exists',
    (reason) => {
      const id = reasonStringId(reason);
      expect(id).not.toBe('client.join.error_unknown');
      expect(STRINGS.get(id)).toBeTruthy();
    },
  );

  it('resolves every id the panel view model can name', () => {
    const ids = new Set(['server.fleet.open', 'server.fleet.closed', 'server.fleet.frozen',
      'server.fleet.slot_owner', 'server.fleet.slot_member', 'server.fleet.no_ship',
      'server.fleet.disconnected', 'server.fleet.ready']);
    for (const id of ids) expect(STRINGS.get(id)).toBeTruthy();
  });

  it('words a fleet refusal for the surface it is read on, not the phone\'s', () => {
    // Every refusal a fleet can produce reaches an operator through
    // `server.fleet.error_joining`, and the phone's wording for several of them
    // is not merely terse but wrong: a ship host that types the lead's CREW
    // code into the fleet field is not being told "that is a fleet code".
    for (const reason of serverSurfaceReasons()) {
      const onPhone = reasonStringId(reason, SURFACE_CLIENT);
      const onHost = reasonStringId(reason, SURFACE_SERVER);
      expect(onHost, reason).not.toBe(onPhone);
      expect(onHost, reason).toMatch(/^server\.fleet\./);
      expect(STRINGS.get(onHost), onHost).toBeTruthy();
    }
    // The inversion that motivated it, stated as the two sentences it is.
    expect(STRINGS.get(reasonStringId('wrong-type', SURFACE_CLIENT))).toContain('fleet code');
    expect(STRINGS.get(reasonStringId('wrong-type', SURFACE_SERVER))).toContain('crew code');
  });

  it('leaves surface-independent refusals with exactly one wording', () => {
    // A second copy of "A join code is five letters." is a second thing to
    // keep true. Only the reasons whose wording DEPENDS on the surface are
    // listed, and the rest fall through to the one map.
    for (const reason of ['empty', 'length', 'charset', 'unreachable', 'malformed']) {
      expect(reasonStringId(reason, SURFACE_SERVER), reason)
        .toBe(reasonStringId(reason, SURFACE_CLIENT));
    }
  });
});
