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
  ADMISSION_CLOSED,
  ADMISSION_OPEN,
  REASON_ADMISSION_CLOSED,
  REASON_FLEET_FULL,
  REASON_RECOVERY_ONLY,
  hostFrame,
  encodeHostFrame,
  decodeHostFrame,
  isHostFrame,
  openFleet,
  admitHost,
  setAdmission,
  freezeFleet,
  updateSlot,
  dropHost,
  rosterOf,
  slotForPeer,
  helloFrame,
  welcomeFrame,
  refusedFrame,
  slotFrame,
  rosterFrame,
  admissionFrame,
  fleetPanelViewModel,
} from '../../gui/host-mesh.js';
import { reasonStringId } from '../../gui/join-code.js';
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
    const frames = [
      helloFrame({ stamp: '1/phoenix-base/1', ship: { template_path: 'a.toml' }, name: 'Two' }),
      welcomeFrame('slot-2', rosterOf(fleetOf())),
      refusedFrame('recovery-only', 'the mission has started'),
      slotFrame({ ship: { template_path: 'b.toml' }, ready: true }),
      rosterFrame(fleetOf()),
      admissionFrame(ADMISSION_CLOSED),
    ];
    for (const frame of frames) {
      expect(HOST_FRAME_TYPES).toContain(frame.t);
      expect(decodeHostFrame(encodeHostFrame(frame))).toEqual(frame);
    }
    // Every declared type is exercised above, so a seventh frame added without
    // a round-trip here fails rather than shipping untested.
    expect(new Set(frames.map((f) => f.t))).toEqual(new Set(HOST_FRAME_TYPES));
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
      'server.fleet.slot_owner', 'server.fleet.slot_member']);
    for (const id of ids) expect(STRINGS.get(id)).toBeTruthy();
  });
});
