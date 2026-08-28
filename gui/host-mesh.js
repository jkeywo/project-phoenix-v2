/**
 * gui/host-mesh.js — the host-to-host vocabulary and the fleet-lobby model
 * (issue #1114).
 *
 * Phoenix has exactly one crew protocol (`ClientMessage`/`ServerMessage`) and,
 * since this module, exactly one HOST protocol. They are deliberately separate:
 * PRD #1093 and `pasm/spec/design/p2p-design-deltas.yaml` both require it, and
 * the reason is structural rather than tidy — a ship host is not a console. It
 * holds no station, claims no seat, is never projected to, and the things it
 * says ("I am a ship in this fleet", "my hull is the destroyer", "admission is
 * closed") have no meaning on a phone.
 *
 * ## The envelope
 *
 *     { m: 1, t: 'hello', tick: null, d: { … } }
 *
 *   `m`     vocabulary revision — {@link HOST_MESH_PROTOCOL}. A frame with any
 *           other value is refused rather than guessed at, exactly as the
 *           rendezvous service refuses a foreign `v`.
 *   `t`     frame type.
 *   `tick`  the logical tick this frame applies at, or `null` for a frame that
 *           is not tick-scoped. NOTHING in #1114 sets it — a fleet lobby has no
 *           running simulation to stamp against — but the field is here from
 *           the first frame because #1116's lockstep work cannot retrofit a
 *           stamp onto a vocabulary already in the field, and because
 *           AGENTS.md rule 7's `CommandDelay` amendment is precisely "the tick
 *           a command applies on travels WITH the command".
 *   `d`     the body.
 *
 * The keys are `m`/`t`/`d` and not `v`/`type`/`data` on purpose. `v` belongs to
 * the rendezvous signalling frames (gui/rendezvous-protocol.js) and `type`/
 * `data` to the crew protocol, so a host-mesh frame that ends up on the wrong
 * wire is REFUSED by whichever decoder receives it instead of being half
 * understood: `decodeHostFrame` rejects anything without an `m`, and every crew
 * decoder in the project switches on `.type`, which a host frame does not have.
 * That is what makes "each crew star remains attached only to its own host"
 * checkable rather than merely intended.
 *
 * ## What v1 carries
 *
 *   member → owner   hello   present a build stamp and a proposed ship
 *                    slot    announce/update THIS member's own slot
 *   owner → member   welcome the handshake verdict, plus the roster
 *                    refused the handshake verdict, with a machine reason
 *   owner → all      roster  full roster sync (admission + freeze + slots)
 *                    admission  admission opened or closed
 *
 * Six frames, and no more: #1116 grows this vocabulary with the lockstep
 * command/tick traffic, and a frame invented here that nothing sends would be
 * surface that work would have to keep.
 *
 * ## What is pure, and why it matters
 *
 * Everything in this file is pure — no DOM, no sockets, no transport. The
 * wiring lives in `gui/fleet-session.js`, the operator surface in `server.html`
 * and `gui/server-settings.js`. So the awkward parts (what a `hello` gets
 * answered with while admission is closed, what happens to a slot when its host
 * drops before versus after mission start) are decided in one tested place.
 */

/**
 * Vocabulary revision. Bump only for an incompatible change — a receiver
 * refuses any other value rather than reading fields that may have moved.
 */
export const HOST_MESH_PROTOCOL = 1;

/** Frame types this revision speaks. */
export const HOST_FRAME_HELLO = 'hello';
export const HOST_FRAME_WELCOME = 'welcome';
export const HOST_FRAME_REFUSED = 'refused';
export const HOST_FRAME_SLOT = 'slot';
export const HOST_FRAME_ROSTER = 'roster';
export const HOST_FRAME_ADMISSION = 'admission';

/** Every type a v1 receiver will accept. Read by the coverage tests. */
export const HOST_FRAME_TYPES = [
  HOST_FRAME_HELLO,
  HOST_FRAME_WELCOME,
  HOST_FRAME_REFUSED,
  HOST_FRAME_SLOT,
  HOST_FRAME_ROSTER,
  HOST_FRAME_ADMISSION,
];

/** Admission states a fleet record can be in — the registry's own two words. */
export const ADMISSION_OPEN = 'open';
export const ADMISSION_CLOSED = 'closed';

// ── The envelope ────────────────────────────────────────────────────────────

/**
 * Wrap a body in the versioned, tick-stampable envelope.
 *
 * @param {string} type one of {@link HOST_FRAME_TYPES}
 * @param {object} [body]
 * @param {{tick?: number|null}} [opts] `tick` is the logical tick this frame
 *   applies at. #1114 never sets it; see the module header.
 */
export function hostFrame(type, body = {}, { tick = null } = {}) {
  return { m: HOST_MESH_PROTOCOL, t: type, tick, d: body };
}

/** Encode one frame for the wire. */
export function encodeHostFrame(frame) {
  return JSON.stringify(frame);
}

/**
 * Decode one wire payload into a frame, or `null`.
 *
 * `null` covers every "this is not a host-mesh frame of a revision I speak"
 * case together — unparseable text, a crew message that arrived on this
 * channel, a rendezvous frame, a future revision. The caller's answer to all of
 * them is the same (drop it), and distinguishing them would invite a receiver
 * to act on a frame it does not understand.
 */
export function decodeHostFrame(raw) {
  const text = typeof raw === 'string' ? raw : null;
  if (text === null) return null;
  let parsed = null;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== 'object') return null;
  if (parsed.m !== HOST_MESH_PROTOCOL) return null;
  if (HOST_FRAME_TYPES.indexOf(parsed.t) < 0) return null;
  return {
    m: parsed.m,
    t: parsed.t,
    tick: typeof parsed.tick === 'number' ? parsed.tick : null,
    d: parsed.d && typeof parsed.d === 'object' ? parsed.d : {},
  };
}

/** True when a decoded payload is a host-mesh frame rather than crew traffic. */
export function isHostFrame(value) {
  return !!value && typeof value === 'object' && value.m === HOST_MESH_PROTOCOL;
}

// ── The fleet model ─────────────────────────────────────────────────────────

/**
 * Refusal reasons this module produces, mapped to the same machine-reason
 * vocabulary `gui/join-code.js`'s `reasonStringId` already speaks, so one
 * failure never gets two wordings. Stamp refusals are NOT here: those are the
 * transport-plane `JoinHandshake` gate's, answered by Rust
 * (`delivery::check_host_stamp`) before a `hello` is ever read.
 */
export const REASON_ADMISSION_CLOSED = 'admission-closed';
export const REASON_FLEET_FULL = 'fleet-full';
/**
 * The post-mission-start answer. Deliberately NOT `admission-closed`: the fleet
 * did not decline this host, it declined to create a *new slot* for it. The
 * server code stays a recovery capability for a slot that is already in the
 * roster and currently disconnected — `p2p-fixed-host-slot-recovery` — and
 * performing that recovery is #1120's, not this issue's. A distinct reason is
 * what lets #1120 land as a change of answer rather than a change of protocol.
 */
export const REASON_RECOVERY_ONLY = 'recovery-only';

/** How a slot id is spelled. */
const slotId = (seq) => `slot-${seq}`;

/**
 * Open a fleet, with this host as its owner.
 *
 * The owner is the host that minted the server code — there is no election and
 * no migration in #1114. That is also why slot ids are minted HERE, from one
 * monotonic counter on one machine: `p2p-delta-identity-is-minted` says an
 * identifier is a function of who minted it and in what order, so a fleet whose
 * members numbered their own slots would produce ids no other host agrees with
 * the moment #1116 needs them in a shared command stream.
 *
 * @param {object} opts
 * @param {{template_path: string|null, name?: string}|null} [opts.ship]
 * @param {string} [opts.name] display name for the owner's own ship slot
 * @param {number} opts.maxSlots authored fleet capacity — see
 *   `assets/join/join-codes.toml` `[limits] max_fleet_hosts`. Required rather
 *   than defaulted: a capacity invented here would be a gameplay number in
 *   code (AGENTS.md rule 11).
 */
export function openFleet({ ship = null, name = '', maxSlots }) {
  return {
    owner: slotId(1),
    nextSeq: 2,
    maxSlots,
    admission: ADMISSION_OPEN,
    frozen: false,
    slots: [
      {
        id: slotId(1),
        peer: null,
        owner: true,
        connected: true,
        ready: false,
        name,
        ship,
      },
    ],
  };
}

/** The slot a rendezvous peer id holds, or null. */
export function slotForPeer(fleet, peer) {
  return fleet.slots.find((s) => s.peer && s.peer === peer) || null;
}

/** The slot with this id, or null. */
export function slotById(fleet, id) {
  return fleet.slots.find((s) => s.id === id) || null;
}

/**
 * Judge a `hello` and, when it passes, seat the host in a new slot.
 *
 * Order matters. Freeze is answered BEFORE closure because they are different
 * facts about the fleet and the operator's next move differs: a closed fleet
 * can be reopened, a frozen one cannot until the mission ends. Capacity is
 * answered last because it is the only one a departing host can change.
 *
 * A peer that already holds a slot is answered with that slot rather than a
 * second one — a duplicate `hello` is noise, not a request for another ship,
 * and the same reasoning the crew path applies to a repeated `JoinHandshake`.
 *
 * @returns {{ok: true, fleet: object, slot: object}
 *          |{ok: false, reason: string}}
 */
export function admitHost(fleet, { peer, ship = null, name = '' }) {
  const held = slotForPeer(fleet, peer);
  if (held) return { ok: true, fleet, slot: held };
  if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  if (fleet.admission !== ADMISSION_OPEN) {
    return { ok: false, reason: REASON_ADMISSION_CLOSED };
  }
  if (fleet.slots.length >= fleet.maxSlots) {
    return { ok: false, reason: REASON_FLEET_FULL };
  }
  const slot = {
    id: slotId(fleet.nextSeq),
    peer,
    owner: false,
    connected: true,
    ready: false,
    name,
    ship,
  };
  return {
    ok: true,
    slot,
    fleet: { ...fleet, nextSeq: fleet.nextSeq + 1, slots: [...fleet.slots, slot] },
  };
}

/**
 * Open or close admission of NEW hosts.
 *
 * It says nothing about the hosts already in the roster, and it may not: an
 * admitted host's link to the owner is a direct DataChannel that owes the
 * signalling plane nothing, which is the same two-planes property the crew
 * transport keeps (see gui/rendezvous-transport.js). Closing a fleet is an
 * answer to future joiners, not an eviction.
 */
export function setAdmission(fleet, state) {
  const next = state === ADMISSION_CLOSED ? ADMISSION_CLOSED : ADMISSION_OPEN;
  return next === fleet.admission ? fleet : { ...fleet, admission: next };
}

/**
 * Mission start: the slot roster and every selected loadout become immutable.
 *
 * Closing admission as well is not redundant. Admission is the operator's
 * lever and can be reopened; the freeze cannot, and a fleet that read "open"
 * over a frozen roster would be advertising a slot it can no longer create.
 */
export function freezeFleet(fleet) {
  if (fleet.frozen) return fleet;
  return { ...fleet, frozen: true, admission: ADMISSION_CLOSED };
}

/**
 * Apply a member's own `slot` announcement.
 *
 * Refused once frozen — that IS the freeze, on the loadout half. `ready` and
 * `ship` are the only fields a member may write about itself; identity
 * (`id`, `peer`, `owner`) belongs to the owner that minted the slot.
 *
 * @returns {{ok: true, fleet: object}|{ok: false, reason: string}}
 */
export function updateSlot(fleet, id, patch = {}) {
  const slot = slotById(fleet, id);
  if (!slot) return { ok: false, reason: 'unknown' };
  if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  const next = { ...slot };
  if (Object.prototype.hasOwnProperty.call(patch, 'ship')) next.ship = patch.ship;
  if (Object.prototype.hasOwnProperty.call(patch, 'ready')) next.ready = !!patch.ready;
  if (Object.prototype.hasOwnProperty.call(patch, 'name')) next.name = String(patch.name || '');
  return {
    ok: true,
    fleet: { ...fleet, slots: fleet.slots.map((s) => (s.id === id ? next : s)) },
  };
}

/**
 * A member host's link ended.
 *
 * Before mission start the slot GOES: the topology is still mutable, and a
 * roster carrying ships nobody is flying would be a lie the operator has to
 * mentally filter. After the freeze the slot STAYS and is marked disconnected,
 * because that is precisely the object #1120's recovery claims — the same
 * non-pruning discipline `SessionManager` keeps for exactly the same reason
 * (`src/lobby/session.rs`: a record that is deleted cannot be reconnected to).
 *
 * The owner's own slot is never dropped; a fleet without its owner is not a
 * fleet, and host migration is #1120's.
 */
export function dropHost(fleet, peer) {
  const slot = slotForPeer(fleet, peer);
  if (!slot || slot.owner) return fleet;
  if (!fleet.frozen) {
    return { ...fleet, slots: fleet.slots.filter((s) => s.id !== slot.id) };
  }
  return {
    ...fleet,
    slots: fleet.slots.map((s) =>
      s.id === slot.id ? { ...s, connected: false, peer: null } : s,
    ),
  };
}

/**
 * The roster body carried by a `roster`/`welcome` frame: everything a member
 * needs to draw the shared fleet lobby, and nothing about the transport.
 *
 * Peer ids are deliberately absent. They are this owner's rendezvous
 * bookkeeping — ephemeral, meaningless on another machine, and the sort of
 * cross-host identity leak `p2p-human-readable-join-namespaces` keeps out of
 * everything but the service itself.
 */
export function rosterOf(fleet) {
  return {
    owner: fleet.owner,
    admission: fleet.admission,
    frozen: fleet.frozen,
    max_slots: fleet.maxSlots,
    slots: fleet.slots.map((s) => ({
      id: s.id,
      owner: s.owner,
      connected: s.connected,
      ready: s.ready,
      name: s.name,
      ship: s.ship,
    })),
  };
}

// ── Frame builders ──────────────────────────────────────────────────────────
//
// One builder per frame so a sender never spells a type string by hand, and so
// the body shape of each frame is written down exactly once.

export const helloFrame = ({ stamp, ship = null, name = '' }) =>
  hostFrame(HOST_FRAME_HELLO, { stamp, ship, name });

export const welcomeFrame = (slotIdent, roster) =>
  hostFrame(HOST_FRAME_WELCOME, { slot: slotIdent, roster });

export const refusedFrame = (code, detail = '') =>
  hostFrame(HOST_FRAME_REFUSED, { code, detail });

export const slotFrame = (patch) => hostFrame(HOST_FRAME_SLOT, patch);

export const rosterFrame = (fleet) => hostFrame(HOST_FRAME_ROSTER, { roster: rosterOf(fleet) });

export const admissionFrame = (state) => hostFrame(HOST_FRAME_ADMISSION, { state });

// ── View model ──────────────────────────────────────────────────────────────

/**
 * What the fleet panel draws, from a roster body — the shape a member has and
 * the owner can build with {@link rosterOf}, so both ends render the same lobby
 * from the same data rather than the owner rendering privileged state.
 *
 * Text that depends on the data is returned as `{id, params}` for the caller to
 * resolve through `t()`, the convention `gui/host-lobby-view.js` already uses.
 *
 * @param {object|null} roster
 * @param {{suffix?: string|null, mine?: string|null}} [opts] `suffix` is the
 *   five-letter server code (owner only); `mine` is this host's own slot id.
 */
export function fleetPanelViewModel(roster, { suffix = null, mine = null } = {}) {
  if (!roster) return { visible: false, rows: [], code: null, status: null };
  const rows = (roster.slots || []).map((s) => ({
    id: s.id,
    mine: !!mine && s.id === mine,
    owner: !!s.owner,
    connected: s.connected !== false,
    ready: !!s.ready,
    name: s.name || '',
    // A slot whose host has not chosen a hull yet is a real state — the
    // operator opened the fleet before picking a ship — so it is rendered as
    // "no ship yet" rather than blank.
    ship: (s.ship && (s.ship.name || s.ship.template_path)) || null,
    label: {
      id: s.owner ? 'server.fleet.slot_owner' : 'server.fleet.slot_member',
      params: { n: String(s.id).replace(/^slot-/, '') },
    },
  }));
  const status = roster.frozen
    ? { id: 'server.fleet.frozen', params: {} }
    : roster.admission === ADMISSION_CLOSED
      ? { id: 'server.fleet.closed', params: {} }
      : {
          id: 'server.fleet.open',
          params: { n: String(rows.length), max: String(roster.max_slots || rows.length) },
        };
  return {
    visible: true,
    code: suffix || null,
    frozen: !!roster.frozen,
    admission: roster.admission || ADMISSION_OPEN,
    rows,
    status,
  };
}

// Expose for classic-script consumers (server.html is not a module).
if (typeof window !== 'undefined') {
  window.hostMesh = {
    HOST_MESH_PROTOCOL,
    HOST_FRAME_TYPES,
    ADMISSION_OPEN,
    ADMISSION_CLOSED,
    REASON_ADMISSION_CLOSED,
    REASON_FLEET_FULL,
    REASON_RECOVERY_ONLY,
    hostFrame,
    encodeHostFrame,
    decodeHostFrame,
    isHostFrame,
    openFleet,
    slotForPeer,
    slotById,
    admitHost,
    setAdmission,
    freezeFleet,
    updateSlot,
    dropHost,
    rosterOf,
    helloFrame,
    welcomeFrame,
    refusedFrame,
    slotFrame,
    rosterFrame,
    admissionFrame,
    fleetPanelViewModel,
  };
}
