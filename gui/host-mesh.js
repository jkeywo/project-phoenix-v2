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
 *   member → owner   hello   present a proposed ship and name
 *                    slot    announce/update THIS member's own slot
 *   owner → member   welcome the handshake verdict, plus the roster
 *                    refused a machine reason, and WHICH frame it answers
 *   owner → all      roster  full roster sync (admission + freeze + slots)
 *                    admission  admission opened or closed
 *
 * ## What revision 2 adds (issue #1116)
 *
 * The running-mission half. Where revision 1 assembles a fleet and freezes it,
 * these are what the frozen fleet says to itself while it plays:
 *
 *   host → all       tick    one host's commands for its own crew, plus the
 *                            watermark saying it will never issue anything for
 *                            those ticks again
 *                    digest  a sampled authoritative fold, for agreement
 *
 * Neither is built or read here — both are OPAQUE to this module, and that is
 * deliberate. Their bodies are minted and consumed by the Rust simulation
 * (`src/lockstep/frame.rs`, encoded in `core::codec`), because what is inside a
 * `tick` frame is a command the authority gate already accepted and a tick the
 * fixed clock already stamped. JavaScript that could construct one could
 * construct a command no host admitted. So this module knows the two frame
 * types exist, refuses anything else, and ferries them — the same relationship
 * `gui/connection-manager.js` has with `ClientMessage`.
 *
 * Eight frames, and no more: a frame invented here that nothing sends would be
 * surface #1117–#1120 would have to keep.
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
 *
 * **Declared twice on purpose.** `lockstep::frame::HOST_MESH_PROTOCOL` carries
 * the same number in Rust, and each half has a test pinning it. A host whose JS
 * speaks 2 and whose Rust speaks 3 would assemble a fleet and then silently
 * fail to agree a tick; refusing an unrecognised `m` is what makes that fail
 * loudly instead, and the pair of pins is what catches a one-sided bump.
 *
 * `3` adds the `host-loss` frame (issue #1119). A revision-2 build that silently
 * DROPPED it would keep waiting for a peer that will never speak again while the
 * revision-3 hosts flipped its ship to Backfill — a split with no symptom, which
 * is exactly why the revision refuses a whole fleet rather than a frame.
 *
 * `4` adds the `slot-claim` frame (issue #1120): the owner announcing that a
 * replacement machine has reclaimed a disconnected fixed slot, so the whole fleet
 * recovers the same one. A revision-3 build that dropped it would keep the ship on
 * Backfill while the revision-4 hosts handed it back — the same silent split.
 */
export const HOST_MESH_PROTOCOL = 4;

/** Frame types this revision speaks. */
export const HOST_FRAME_HELLO = 'hello';
export const HOST_FRAME_WELCOME = 'welcome';
export const HOST_FRAME_REFUSED = 'refused';
export const HOST_FRAME_SLOT = 'slot';
export const HOST_FRAME_ROSTER = 'roster';
export const HOST_FRAME_ADMISSION = 'admission';
/** Revision 2 (issue #1116): one host's input for a tick, and its watermark. */
export const HOST_FRAME_TICK = 'tick';
/** Revision 2 (issue #1116): a sampled authoritative fold, for agreement. */
export const HOST_FRAME_DIGEST = 'digest';
/**
 * Revision 2 (issue #1117): one framed piece of a portable snapshot in transit.
 *
 * Like `tick` and `digest`, its body is minted and read only by Rust — this
 * module never inspects a chunk's `d`, it only ferries it. The chunking,
 * reassembly and integrity live in `src/lockstep/transfer.rs`; the page's job is
 * to carry a chunk to `wasm_receive_mesh_frame` and relay it to any sibling.
 */
export const HOST_FRAME_SNAPSHOT = 'snapshot';
/**
 * Revision 3 (issue #1119): a ship host has left. The body carries the lost
 * slot and the tick its ship flips to Backfill on — the first tick past that
 * host's own last watermark, so every survivor agrees it without arbitrating.
 */
export const HOST_FRAME_HOST_LOSS = 'host-loss';
/**
 * Revision 4 (issue #1120): the owner announcing that a replacement machine has
 * claimed a disconnected fixed slot, so every host recovers the same one. Like the
 * other running-mission frames its body is minted and read only by Rust; this
 * module ferries it opaquely.
 */
export const HOST_FRAME_SLOT_CLAIM = 'slot-claim';

/** Every type a receiver will accept. Read by the coverage tests. */
export const HOST_FRAME_TYPES = [
  HOST_FRAME_HELLO,
  HOST_FRAME_WELCOME,
  HOST_FRAME_REFUSED,
  HOST_FRAME_SLOT,
  HOST_FRAME_ROSTER,
  HOST_FRAME_ADMISSION,
  HOST_FRAME_TICK,
  HOST_FRAME_DIGEST,
  HOST_FRAME_SNAPSHOT,
  HOST_FRAME_HOST_LOSS,
  HOST_FRAME_SLOT_CLAIM,
];

/**
 * The frames the SIMULATION owns — minted and read by Rust, ferried by this
 * module and never inspected by it (issue #1116).
 *
 * The split matters at exactly one place: {@link isSimulationFrame} is what
 * `gui/fleet-session.js` uses to decide whether a decoded frame goes to the
 * fleet model here or straight across the wasm boundary. A lobby frame that
 * reached the simulation would be a roster edit nobody admitted; a tick frame
 * that reached the fleet model would be silently dropped and the fleet would
 * stall on the peer that sent it.
 */
export const HOST_SIMULATION_FRAME_TYPES = [
  HOST_FRAME_TICK,
  HOST_FRAME_DIGEST,
  HOST_FRAME_SNAPSHOT,
  HOST_FRAME_HOST_LOSS,
  HOST_FRAME_SLOT_CLAIM,
];

/** True when this frame belongs to the running simulation rather than the lobby. */
export function isSimulationFrame(frame) {
  return !!frame && HOST_SIMULATION_FRAME_TYPES.indexOf(frame.t) >= 0;
}

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
  if (typeof raw !== 'string') return null;
  try {
    return asHostFrame(JSON.parse(raw));
  } catch {
    return null;
  }
}

/**
 * The same judgement over an ALREADY-PARSED value, for a receiver that was
 * handed an object rather than text — which is what the transport's own
 * `onData` delivers. Re-serialising just to re-parse it would be a second
 * decode path one careless edit away from disagreeing with this one.
 */
export function asHostFrame(parsed) {
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
/**
 * A claim on a slot that cannot be recovered on another machine (issue #1120):
 * it does not exist, its host is still connected, or it is the owner's own slot.
 * Distinct from `recovery-only` — that answers a host asking for a NEW slot after
 * the freeze; this answers a host asking to reclaim a specific one it may not.
 * "Cannot displace a connected host" is exactly this reason for a live slot.
 */
export const REASON_SLOT_TAKEN = 'slot-taken';

/** How a slot id is spelled. */
const slotId = (seq) => `slot-${seq}`;

/**
 * The ordinal `N` in a `slot-N` id, or `null` for anything that is not one.
 *
 * The one number the simulation orders and routes on
 * (`command_admission::log::HostSlot`), so a host-loss report (issue #1119) has
 * to reduce a slot id to it before crossing the wasm boundary. `null` rather
 * than a guess for a malformed id, for the same reason `HostSlot::from_slot_id`
 * answers `None`: a slot this side cannot parse is a protocol disagreement, not
 * a slot to invent.
 */
export function hostSlotOrdinal(id) {
  if (typeof id !== 'string') return null;
  const m = /^slot-(\d+)$/.exec(id);
  return m ? Number(m[1]) : null;
}

/**
 * Fallback bounds on what a member may say about itself.
 *
 * Authored in `assets/join/join-codes.toml` `[limits]` alongside the registry's
 * own bounds and `max_fleet_hosts`; these are the parse-time defaults an older
 * table still loads under (AGENTS.md rule 11a), NOT a second answer. They are
 * protocol hygiene rather than a gameplay number: a name and a hull path are
 * the only two fields one host may write into every other host's roster, and
 * `publish()` re-encodes that roster to the whole fleet on every change.
 */
const DEFAULT_MAX_NAME_LENGTH = 48;
const DEFAULT_MAX_SHIP_PATH_LENGTH = 160;

/** A bounded, plain string, or '' for anything that is not one. */
function boundedText(value, limit) {
  if (typeof value !== 'string') return '';
  return value.slice(0, limit);
}

/**
 * A member's proposed ship, reduced to the two fields a roster carries.
 *
 * Anything else — a nested object, an array, a megabyte of text, extra keys —
 * is dropped rather than stored, because whatever enters here is re-broadcast
 * to every admitted host on every roster change. The same discipline the
 * rendezvous registry applies to the one host-supplied value it indexes on
 * (`RELEASE_GUID`, worker-rendezvous/src/registry.js): a peer-supplied field
 * that reaches other peers gets a shape, not a cast.
 *
 * `null` is a real answer — "this host has not chosen a hull yet" — and is kept
 * as one; a ship with no usable `template_path` collapses to it.
 */
function boundedShip(value, fleet) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const templatePath = boundedText(value.template_path, fleet.maxShipPathLength);
  if (!templatePath) return null;
  const name = boundedText(value.name, fleet.maxNameLength);
  return name ? { template_path: templatePath, name } : { template_path: templatePath };
}

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
 * @param {number} [opts.maxNameLength] `[limits] max_slot_name_length`
 * @param {number} [opts.maxShipPathLength] `[limits] max_slot_ship_path_length`
 */
export function openFleet({
  ship = null,
  name = '',
  maxSlots,
  maxNameLength = DEFAULT_MAX_NAME_LENGTH,
  maxShipPathLength = DEFAULT_MAX_SHIP_PATH_LENGTH,
}) {
  const fleet = {
    owner: slotId(1),
    nextSeq: 2,
    maxSlots,
    maxNameLength,
    maxShipPathLength,
    admission: ADMISSION_OPEN,
    frozen: false,
    slots: [],
  };
  fleet.slots.push({
    id: slotId(1),
    peer: null,
    owner: true,
    connected: true,
    ready: false,
    // The owner's own fields go through the same bound as a member's. It is
    // this host's own page filling them in, so nothing hostile is expected —
    // but one rule for what a slot may hold is easier to keep true than two.
    name: boundedText(name, maxNameLength),
    ship: boundedShip(ship, fleet),
  });
  return fleet;
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
    // Bounded and shape-checked at the door. `name` and `ship` are the only
    // two fields a member writes about itself, and they land in every other
    // host's roster on the next publish.
    name: boundedText(name, fleet.maxNameLength),
    ship: boundedShip(ship, fleet),
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
 * Refused once frozen — that IS the freeze, on the loadout half. `ready`,
 * `ship` and `name` are the only fields a member may write about itself;
 * identity (`id`, `peer`, `owner`) belongs to the owner that minted the slot,
 * and what it may write is bounded and shape-checked on the way in.
 *
 * @returns {{ok: true, fleet: object}|{ok: false, reason: string}}
 */
export function updateSlot(fleet, id, patch = {}) {
  const slot = slotById(fleet, id);
  if (!slot) return { ok: false, reason: 'unknown' };
  if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  const next = { ...slot };
  if (Object.prototype.hasOwnProperty.call(patch, 'ship')) {
    next.ship = boundedShip(patch.ship, fleet);
  }
  if (Object.prototype.hasOwnProperty.call(patch, 'ready')) next.ready = !!patch.ready;
  if (Object.prototype.hasOwnProperty.call(patch, 'name')) {
    next.name = boundedText(patch.name, fleet.maxNameLength);
  }
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
 * A replacement machine reclaiming ONE disconnected fixed slot (issue #1120).
 *
 * The recovery half of `admitHost`: where a post-freeze `hello` for a NEW slot is
 * answered `recovery-only`, a `hello` that names a specific slot to CLAIM is judged
 * here. It is the whole of AC1's "server-code entry can select ONLY a disconnected
 * fixed ship slot and cannot add a ship, change its loadout or displace a connected
 * host":
 *
 * * it applies only after the freeze — before it, a reconnecting host is admitted
 *   the ordinary way and there is no fixed slot to recover;
 * * the slot must EXIST and must be currently disconnected — a claim on a live slot
 *   is refused `slot-taken`, so a connected host is never displaced;
 * * the owner's own slot is never reclaimable here (host migration is out of scope),
 *   also `slot-taken`;
 * * the frozen ship and crew are KEPT verbatim — the claim carries no loadout, so a
 *   replacement resumes the same ship it is recovering and cannot change it, and a
 *   later `slot` patch is refused by `updateSlot`'s freeze exactly as any member's is.
 *
 * On success the slot's `peer` is rebound to the claiming connection and it is
 * marked connected again; the caller (`gui/fleet-session.js`) welcomes the
 * replacement as that slot and broadcasts the fleet-wide grant.
 *
 * @returns {{ok: true, fleet: object, slot: object}
 *          |{ok: false, reason: string}}
 */
export function claimSlot(fleet, { peer, slotId }) {
  if (!fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  const slot = slotById(fleet, slotId);
  if (!slot) return { ok: false, reason: 'unknown' };
  // The owner's own slot is never recovered on another machine, and a slot whose
  // host is still connected is never displaced.
  if (slot.owner || slot.connected) return { ok: false, reason: REASON_SLOT_TAKEN };
  const next = { ...slot, peer, connected: true };
  return {
    ok: true,
    slot: next,
    fleet: { ...fleet, slots: fleet.slots.map((s) => (s.id === slotId ? next : s)) },
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

/**
 * A member introducing itself.
 *
 * It carries NO delivery stamp. The build check is the transport plane's, made
 * by `delivery::check_host_stamp` over the in-band `JoinHandshake` before a
 * `hello` is ever read, and a copy of an authoritative fact — peer-supplied,
 * unvalidated, sitting in the frame a future reader reaches for first — is
 * precisely how a second, weaker check gets written by accident.
 */
export const helloFrame = ({ ship = null, name = '', claim = null }) =>
  // `claim` (issue #1120) is the slot id a replacement machine is reclaiming; it
  // is absent for an ordinary join and only acted on after the freeze. Kept off
  // the body entirely when null, so a pre-#1120 lead sees an unchanged hello.
  hostFrame(HOST_FRAME_HELLO, claim ? { ship, name, claim } : { ship, name });

export const welcomeFrame = (slotIdent, roster) =>
  hostFrame(HOST_FRAME_WELCOME, { slot: slotIdent, roster });

/**
 * A refusal, and WHICH frame it answers.
 *
 * `of` is load-bearing rather than diagnostic. The owner refuses from two very
 * different places — a `hello` from a host it will not seat, and a `slot` patch
 * from a host that is already seated — and a member that cannot tell them apart
 * has to treat both as terminal. That would throw a legitimate member out of
 * its own fleet for touching its loadout after the freeze. The subject is what
 * makes "your join was refused" and "that change was refused" two answers.
 *
 * @param {string} code machine reason
 * @param {{of?: string, detail?: string}} [opts] `of` is the frame type being
 *   answered — {@link HOST_FRAME_HELLO} or {@link HOST_FRAME_SLOT}.
 */
export const refusedFrame = (code, { of = HOST_FRAME_HELLO, detail = '' } = {}) =>
  hostFrame(HOST_FRAME_REFUSED, { code, of, detail });

export const slotFrame = (patch) => hostFrame(HOST_FRAME_SLOT, patch);

export const rosterFrame = (fleet) => hostFrame(HOST_FRAME_ROSTER, { roster: rosterOf(fleet) });

export const admissionFrame = (state) => hostFrame(HOST_FRAME_ADMISSION, { state });

/**
 * Wrap an already-encoded simulation frame body from the Rust side.
 *
 * There is deliberately no builder for a `tick` or `digest` BODY here. The
 * simulation mints both (`core::codec::encode_mesh_frame`), and JavaScript that
 * could construct a tick frame could construct a command no authority gate ever
 * accepted — a fleet's own hosts are trusted for exactly what they admitted and
 * nothing more. What this module owns is the envelope, which is why the two
 * pass through it rather than round it.
 *
 * @param {string} type a member of {@link HOST_SIMULATION_FRAME_TYPES} —
 *   {@link HOST_FRAME_TICK}, {@link HOST_FRAME_DIGEST} or {@link HOST_FRAME_SNAPSHOT}
 *   (#1117 added the third, built through this same envelope)
 * @param {object} body the body Rust encoded
 * @param {number|null} tick the tick this frame applies at — the field #1114
 *   put in the envelope for exactly this, finally set
 */
export function simulationFrame(type, body, tick = null) {
  return hostFrame(type, body, { tick });
}

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
 *   server code (owner only); `mine` is this host's own slot id.
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
    HOST_SIMULATION_FRAME_TYPES,
    HOST_FRAME_TICK,
    HOST_FRAME_DIGEST,
    HOST_FRAME_SNAPSHOT,
    HOST_FRAME_HOST_LOSS,
    HOST_FRAME_SLOT_CLAIM,
    isSimulationFrame,
    simulationFrame,
    hostSlotOrdinal,
    ADMISSION_OPEN,
    ADMISSION_CLOSED,
    REASON_ADMISSION_CLOSED,
    REASON_FLEET_FULL,
    REASON_RECOVERY_ONLY,
    REASON_SLOT_TAKEN,
    claimSlot,
    hostFrame,
    encodeHostFrame,
    decodeHostFrame,
    asHostFrame,
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
