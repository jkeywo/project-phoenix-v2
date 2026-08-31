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
 * ## What revisions 6 and 7 add (issue #1290)
 *
 *   host → owner      start-state   role-local crew/GM readiness + validation
 *   GM → owner        start-force   empty request; identity is the connection
 *   owner → all       start-policy  one pure aggregate judgement
 *                     force-result  attributed applied/no-op/refused feedback
 *
 * These are control frames, not simulation frames. JavaScript owns the fleet
 * roster policy, but the resulting grant is delivered only to the owner's local
 * Rust peer. Rust schedules it through the deterministic mesh; there is no
 * asynchronous JavaScript `start-grant` broadcast for members to replay.
 * Revision 7 also carries the connected technical participant slots separately
 * from ship rows so a GM participates in lockstep without consuming a ship.
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
 *
 * `5` adds the host role and GM reconnect identity (issue #1289). A GM is a
 * deterministic host-mesh peer but not a ship: it owns a private technical
 * `slot-N` for frame authentication and a separate public `gm-N` operator id.
 * A revision-4 host would otherwise silently turn that peer into a player ship.
 *
 * `6` added the collective GM/crew start policy (issue #1290). Crew readiness,
 * GM readiness, validation and force requests are dedicated host-control
 * frames. `7` removes the asynchronous JavaScript start-grant broadcast and
 * adds the connected technical participant set to the frozen roster. A
 * revision-6 host would omit GM peers from Rust's lockstep wait-set and could
 * apply the decision before every deterministic peer reached its tick.
 */
export const HOST_MESH_PROTOCOL = 7;

/** Frame types this revision speaks. */
export const HOST_FRAME_HELLO = 'hello';
export const HOST_FRAME_WELCOME = 'welcome';
export const HOST_FRAME_REFUSED = 'refused';
export const HOST_FRAME_SLOT = 'slot';
export const HOST_FRAME_ROSTER = 'roster';
export const HOST_FRAME_ADMISSION = 'admission';
/** Revision 6 (#1290): one authenticated host's local readiness/validation. */
export const HOST_FRAME_START_STATE = 'start-state';
/** Revision 6 (#1290): the star owner's pure aggregate policy projection. */
export const HOST_FRAME_START_POLICY = 'start-policy';
/** Revision 6 (#1290): an authenticated GM asks to bypass readiness only. */
export const HOST_FRAME_START_FORCE = 'start-force';
/** Revision 6 (#1290): attributed applied/no-op/refused force feedback. */
export const HOST_FRAME_FORCE_RESULT = 'force-result';
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
  HOST_FRAME_START_STATE,
  HOST_FRAME_START_POLICY,
  HOST_FRAME_START_FORCE,
  HOST_FRAME_FORCE_RESULT,
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

/** Host roles carried only by the privileged host-mesh hello. */
export const HOST_ROLE_SHIP = 'ship';
export const HOST_ROLE_GM = 'gm';

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

/** How a public GM operator id is spelled. It conveys no ordering authority. */
const gmId = (seq) => `gm-${seq}`;

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
const MAX_RECONNECT_CREDENTIAL_LENGTH = 256;
/** Protocol/memory ceiling matching one rendezvous record, not ship capacity. */
export const MAX_GM_OPERATORS = 32;
/** Matches Rust `ReadinessTally`'s u32 aggregate; protocol hygiene, not gameplay. */
const MAX_READINESS_AGGREGATE = 0xffff_ffff;

/** A bounded, plain string, or '' for anything that is not one. */
function boundedText(value, limit) {
  if (typeof value !== 'string') return '';
  return value.slice(0, limit);
}

/** Only the two roles this protocol speaks; absence remains the legacy ship role. */
function hostRole(value) {
  return value === HOST_ROLE_GM ? HOST_ROLE_GM : HOST_ROLE_SHIP;
}

/**
 * Mint an opaque reconnect capability. There is deliberately no predictable
 * fallback: a runtime without cryptographic randomness may not issue a GM
 * identity that another peer could guess.
 */
function secureReconnectCredential() {
  const cryptoApi = typeof globalThis !== 'undefined' ? globalThis.crypto : null;
  if (cryptoApi && typeof cryptoApi.randomUUID === 'function') {
    return cryptoApi.randomUUID();
  }
  if (cryptoApi && typeof cryptoApi.getRandomValues === 'function') {
    const bytes = new Uint8Array(24);
    cryptoApi.getRandomValues(bytes);
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  }
  throw new Error('secure randomness is required to mint a GM reconnect credential');
}

/** Reduce an untrusted reconnect claim to a bounded string before comparing it. */
function reconnectClaim(value) {
  if (typeof value !== 'string' || !value || value.length > MAX_RECONNECT_CREDENTIAL_LENGTH) {
    return null;
  }
  return value;
}

/**
 * A crew tally is produced by the authoritative Rust SessionManager on one
 * ship host. Spectators have already been excluded there. Reduce the crossing
 * to two safe non-negative integers and never permit `ready > connected`.
 */
function crewReadiness(value, maxSlots = 1) {
  const source = value && typeof value === 'object' ? value : {};
  // At most `maxSlots` ship tallies and MAX_GM_OPERATORS one-vote rows are
  // aggregated. Dividing Rust's u32 aggregate ceiling across that authored/
  // protocol-bounded population guarantees the final policy still fits the
  // typed Rust contract even when an authenticated but faulty host sends an
  // absurd tally.
  const slots = Number.isSafeInteger(maxSlots) && maxSlots > 0 ? maxSlots : 1;
  const cap = Math.floor(MAX_READINESS_AGGREGATE / (slots + MAX_GM_OPERATORS));
  const count = (candidate) =>
    Number.isSafeInteger(candidate) && candidate >= 0 ? Math.min(candidate, cap) : 0;
  const connected = count(source.connected);
  return { connected, ready: Math.min(count(source.ready), connected) };
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
  role = HOST_ROLE_SHIP,
  maxSlots,
  maxNameLength = DEFAULT_MAX_NAME_LENGTH,
  maxShipPathLength = DEFAULT_MAX_SHIP_PATH_LENGTH,
  credentialFactory = secureReconnectCredential,
}) {
  const ownerRole = hostRole(role);
  const fleet = {
    // Technical star-centre identity. It is not a public GM leader marker.
    owner: slotId(1),
    nextSeq: 2,
    nextGmSeq: 1,
    maxSlots,
    maxNameLength,
    maxShipPathLength,
    credentialFactory,
    admission: ADMISSION_OPEN,
    frozen: false,
    nextStartSeq: 1,
    startGrant: null,
    slots: [],
    gms: [],
  };
  if (ownerRole === HOST_ROLE_GM) {
    fleet.gms.push({
      id: gmId(fleet.nextGmSeq),
      meshSlot: fleet.owner,
      peer: null,
      connected: true,
      ready: false,
      startValidation: false,
      name: boundedText(name, maxNameLength),
      credential: reconnectClaim(credentialFactory()),
    });
    if (!fleet.gms[0].credential) {
      throw new Error('GM reconnect credential factory returned an unusable value');
    }
    fleet.nextGmSeq += 1;
  } else {
    fleet.slots.push({
      id: slotId(1),
      peer: null,
      owner: true,
      connected: true,
      ready: false,
      crew: crewReadiness(null, fleet.maxSlots),
      startValidation: false,
      // The owner's own fields go through the same bound as a member's. It is
      // this host's own page filling them in, so nothing hostile is expected —
      // but one rule for what a slot may hold is easier to keep true than two.
      name: boundedText(name, maxNameLength),
      ship: boundedShip(ship, fleet),
    });
  }
  return fleet;
}

/** The slot a rendezvous peer id holds, or null. */
export function slotForPeer(fleet, peer) {
  return fleet.slots.find((s) => s.peer && s.peer === peer) || null;
}

/** The private GM record held by a rendezvous peer id, or null. */
export function gmForPeer(fleet, peer) {
  return (fleet.gms || []).find((gm) => gm.peer && gm.peer === peer) || null;
}

/** The private GM record for a public operator id, or null. */
export function gmById(fleet, id) {
  return (fleet.gms || []).find((gm) => gm.id === id) || null;
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
export function admitHost(fleet, {
  peer,
  ship = null,
  name = '',
  role = HOST_ROLE_SHIP,
  reconnectCredential = null,
}) {
  const held = slotForPeer(fleet, peer);
  if (held) {
    return { ok: true, fleet, role: HOST_ROLE_SHIP, slot: held, meshSlot: held.id };
  }
  const heldGm = gmForPeer(fleet, peer);
  if (heldGm) {
    return {
      ok: true,
      fleet,
      role: HOST_ROLE_GM,
      gm: heldGm,
      meshSlot: heldGm.meshSlot,
      operatorId: heldGm.id,
      reconnectCredential: heldGm.credential,
    };
  }

  const nextRole = hostRole(role);
  if (nextRole === HOST_ROLE_GM) {
    const claim = reconnectClaim(reconnectCredential);
    if (reconnectCredential != null) {
      // A supplied credential is a recovery attempt, never permission to mint
      // a second operator after a typo. One answer covers unknown credentials
      // and attempts to displace a still-connected operator, so this boundary
      // does not become a credential-validity oracle.
      const known = claim
        ? (fleet.gms || []).find((gm) => gm.credential === claim)
        : null;
      if (!known || known.connected) {
        return { ok: false, reason: REASON_RECOVERY_ONLY };
      }
      // Once frozen, the technical slot has joined Rust's deterministic
      // wait-set. A departed slot cannot safely reappear without transferring
      // the authoritative snapshot and watermark it missed (including the
      // one-shot mission-start grant), so recovery remains fail-closed until
      // that protocol exists.
      if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
      const rebound = {
        ...known,
        peer,
        connected: true,
        // A disconnected participant never carries a ready vote or a stale
        // local validation verdict into a new transport session.
        ready: false,
        startValidation: false,
      };
      const nextFleet = {
        ...fleet,
        gms: fleet.gms.map((gm) => (gm.id === known.id ? rebound : gm)),
      };
      return {
        ok: true,
        fleet: nextFleet,
        role: HOST_ROLE_GM,
        gm: rebound,
        meshSlot: rebound.meshSlot,
        operatorId: rebound.id,
        reconnectCredential: rebound.credential,
        reconnected: true,
      };
    }
    if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
    if (fleet.admission !== ADMISSION_OPEN) {
      return { ok: false, reason: REASON_ADMISSION_CLOSED };
    }
    if (fleet.gms.length >= MAX_GM_OPERATORS) {
      return { ok: false, reason: REASON_FLEET_FULL };
    }
    const credential = reconnectClaim(fleet.credentialFactory());
    if (!credential) throw new Error('GM reconnect credential factory returned an unusable value');
    if (fleet.gms.some((gm) => gm.credential === credential)) {
      throw new Error('GM reconnect credential factory returned a duplicate value');
    }
    const gm = {
      id: gmId(fleet.nextGmSeq),
      meshSlot: slotId(fleet.nextSeq),
      peer,
      connected: true,
      ready: false,
      startValidation: false,
      name: boundedText(name, fleet.maxNameLength),
      credential,
    };
    return {
      ok: true,
      role: HOST_ROLE_GM,
      gm,
      meshSlot: gm.meshSlot,
      operatorId: gm.id,
      reconnectCredential: gm.credential,
      fleet: {
        ...fleet,
        nextSeq: fleet.nextSeq + 1,
        nextGmSeq: fleet.nextGmSeq + 1,
        gms: [...fleet.gms, gm],
      },
    };
  }

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
    crew: crewReadiness(null, fleet.maxSlots),
    startValidation: false,
    // Bounded and shape-checked at the door. `name` and `ship` are the only
    // two fields a member writes about itself, and they land in every other
    // host's roster on the next publish.
    name: boundedText(name, fleet.maxNameLength),
    ship: boundedShip(ship, fleet),
  };
  return {
    ok: true,
    role: HOST_ROLE_SHIP,
    slot,
    meshSlot: slot.id,
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
 * Replace one ship host's authoritative connected/ready PLAYER tally.
 * `slot.ready` remains the loadout/content-ready bit introduced with the fleet
 * lobby; this separate pair is the crew readiness #1290 aggregates.
 */
export function setCrewReadiness(fleet, id, tally = {}) {
  const slot = slotById(fleet, id);
  if (!slot || !slot.connected) return { ok: false, reason: 'unknown' };
  if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  const next = { ...slot, crew: crewReadiness(tally, fleet.maxSlots) };
  return {
    ok: true,
    fleet: { ...fleet, slots: fleet.slots.map((candidate) => candidate.id === id ? next : candidate) },
  };
}

/** Set one connected GM's own ready vote; no ship slot is involved. */
export function setGmReady(fleet, id, ready) {
  const gm = gmById(fleet, id);
  if (!gm || !gm.connected) return { ok: false, reason: 'unknown' };
  if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  const next = { ...gm, ready: !!ready };
  return {
    ok: true,
    fleet: { ...fleet, gms: fleet.gms.map((candidate) => candidate.id === id ? next : candidate) },
  };
}

/**
 * Set the local start-validation verdict for one authenticated technical mesh
 * slot. Both product roles have one; the slot remains private on a GM row.
 */
export function setStartValidation(fleet, meshSlot, valid) {
  if (fleet.frozen) return { ok: false, reason: REASON_RECOVERY_ONLY };
  const slot = slotById(fleet, meshSlot);
  if (slot && slot.connected) {
    const next = { ...slot, startValidation: !!valid };
    return {
      ok: true,
      fleet: {
        ...fleet,
        slots: fleet.slots.map((candidate) => candidate.id === meshSlot ? next : candidate),
      },
    };
  }
  const gm = (fleet.gms || []).find((candidate) => candidate.meshSlot === meshSlot);
  if (gm && gm.connected) {
    const next = { ...gm, startValidation: !!valid };
    return {
      ok: true,
      fleet: {
        ...fleet,
        gms: fleet.gms.map((candidate) => candidate.id === gm.id ? next : candidate),
      },
    };
  }
  return { ok: false, reason: 'unknown' };
}

/**
 * The one pure global start judgement. The star owner computes and broadcasts
 * it because every member connects there, not because the owner has product
 * authority. Connected player counts come from ship-local Rust tallies;
 * connected GM rows contribute one vote each. A fleet needs at least one
 * participant, every participant ready, every connected host validation true,
 * and every ship loadout/content-ready bit true to autostart.
 */
export function startPolicyOf(fleet) {
  const connectedSlots = fleet.slots.filter((slot) => slot.connected !== false);
  const connectedGms = (fleet.gms || []).filter((gm) => gm.connected !== false);
  const playerCounts = connectedSlots.reduce(
    (sum, slot) => ({
      connected: sum.connected + crewReadiness(slot.crew, fleet.maxSlots).connected,
      ready: sum.ready + crewReadiness(slot.crew, fleet.maxSlots).ready,
    }),
    { connected: 0, ready: 0 },
  );
  const readyGms = connectedGms.filter((gm) => gm.ready).length;
  const connectedTotal = playerCounts.connected + connectedGms.length;
  const readyTotal = playerCounts.ready + readyGms;
  const validationPassed =
    connectedSlots.every((slot) => !!slot.ready && !!slot.startValidation)
    && connectedGms.every((gm) => !!gm.startValidation);
  const allReady = connectedTotal > 0 && readyTotal === connectedTotal;
  const started = !!fleet.startGrant;
  return {
    connected_players: playerCounts.connected,
    ready_players: playerCounts.ready,
    connected_gms: connectedGms.length,
    ready_gms: readyGms,
    connected_total: connectedTotal,
    ready_total: readyTotal,
    all_ready: allReady,
    validation_passed: validationPassed,
    can_auto_start: !started && validationPassed && allReady,
    started,
  };
}

/** Create the fleet's single immutable start grant. Repeats return it unchanged. */
export function grantFleetStart(fleet, { mode = 'automatic', operatorId = null } = {}) {
  if (fleet.startGrant) return { fleet, grant: fleet.startGrant, created: false };
  const forced = mode === 'forced';
  const grant = {
    id: `start-${fleet.nextStartSeq}`,
    mode: forced ? 'forced' : 'automatic',
    operator_id: forced ? operatorId : null,
  };
  return {
    fleet: freezeFleet({
      ...fleet,
      nextStartSeq: fleet.nextStartSeq + 1,
      startGrant: grant,
    }),
    grant,
    created: true,
  };
}

/**
 * Judge a GM force request against current state. Identity is a stable GM id
 * resolved from the authenticated connection by `fleet-session`, never trusted
 * from the request body. Force skips readiness and nothing else.
 */
export function adjudicateForceStart(fleet, operatorId) {
  const gm = gmById(fleet, operatorId);
  if (!gm || !gm.connected) {
    return { ok: false, fleet, result: null };
  }
  if (fleet.startGrant) {
    return {
      ok: true,
      fleet,
      grant: fleet.startGrant,
      result: {
        status: 'no-op',
        operator_id: gm.id,
        reason: 'already-started',
        grant_id: fleet.startGrant.id,
      },
    };
  }
  if (!startPolicyOf(fleet).validation_passed) {
    return {
      ok: true,
      fleet,
      grant: null,
      result: {
        status: 'refused',
        operator_id: gm.id,
        reason: 'validation-failed',
        grant_id: null,
      },
    };
  }
  const granted = grantFleetStart(fleet, { mode: 'forced', operatorId: gm.id });
  return {
    ok: true,
    fleet: granted.fleet,
    grant: granted.grant,
    result: {
      status: 'applied',
      operator_id: gm.id,
      reason: null,
      grant_id: granted.grant.id,
    },
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
  const gm = gmForPeer(fleet, peer);
  if (gm) {
    return {
      ...fleet,
      // GM identities survive every transport drop. The opaque credential is
      // the only capability that may bind this public operator id to a new
      // peer, whether ordinary admission is open or closed.
      gms: fleet.gms.map((candidate) =>
        candidate.id === gm.id
          ? {
              ...candidate,
              connected: false,
              peer: null,
              ready: false,
              startValidation: false,
            }
          : candidate,
      ),
    };
  }
  const slot = slotForPeer(fleet, peer);
  if (!slot || slot.owner) return fleet;
  if (!fleet.frozen) {
    return { ...fleet, slots: fleet.slots.filter((s) => s.id !== slot.id) };
  }
  return {
    ...fleet,
    slots: fleet.slots.map((s) =>
      s.id === slot.id
        ? {
            ...s,
            connected: false,
            peer: null,
            crew: crewReadiness(null, fleet.maxSlots),
            startValidation: false,
          }
        : s,
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
  const participants = [
    ...fleet.slots
      .filter((slot) => slot.connected !== false)
      .map((slot) => slot.id),
    ...(fleet.gms || [])
      .filter((gm) => gm.connected !== false)
      .map((gm) => gm.meshSlot),
  ].filter((slot, index, all) => hostSlotOrdinal(slot) != null && all.indexOf(slot) === index)
    .sort((left, right) => hostSlotOrdinal(left) - hostSlotOrdinal(right));
  return {
    // `owner` is the host-mesh star centre used for deterministic routing. GM
    // rows deliberately carry no corresponding owner/leader/permission bit.
    owner: fleet.owner,
    admission: fleet.admission,
    frozen: fleet.frozen,
    max_slots: fleet.maxSlots,
    // Host-only deterministic topology. Public GM rows deliberately do not
    // reveal which participant slot belongs to which operator identity.
    participants,
    slots: fleet.slots.map((s) => ({
      id: s.id,
      owner: s.owner,
      connected: s.connected,
      ready: s.ready,
      crew: crewReadiness(s.crew, fleet.maxSlots),
      name: s.name,
      ship: s.ship,
    })),
    gms: (fleet.gms || []).map((gm) => ({
      id: gm.id,
      name: gm.name,
      connected: gm.connected,
      ready: !!gm.ready,
    })),
  };
}

/**
 * Convert the frozen host roster into the private numeric schema Rust adopts.
 *
 * Technical participants and player ships are separate on purpose: a GM is a
 * full deterministic peer but owns no ship. Only connected rows are present in
 * `roster.participants`, so every emitted ship host is also a participant.
 * Returns `null` rather than inventing a slot when a foreign/malformed roster
 * cannot name this peer or the star owner.
 */
export function simulationRosterOf(roster, mine) {
  if (!roster || typeof roster !== 'object') return null;
  const local = hostSlotOrdinal(mine);
  const owner = hostSlotOrdinal(roster.owner);
  const participants = Array.from(new Set(
    (Array.isArray(roster.participants) ? roster.participants : [])
      .map(hostSlotOrdinal)
      .filter((slot) => Number.isSafeInteger(slot) && slot > 0),
  )).sort((left, right) => left - right);
  if (local == null || owner == null
      || !participants.includes(local) || !participants.includes(owner)) return null;

  const participantSet = new Set(participants);
  const ships = (Array.isArray(roster.slots) ? roster.slots : [])
    .filter((slot) => slot && slot.connected !== false)
    .map((slot) => ({
      host: hostSlotOrdinal(slot.id),
      ship_path: slot.ship && typeof slot.ship.template_path === 'string'
        ? slot.ship.template_path
        : null,
      crew: [],
    }))
    .filter((ship) => Number.isSafeInteger(ship.host) && participantSet.has(ship.host))
    .sort((left, right) => left.host - right.host);

  return { local, owner, participants, ships };
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
export const helloFrame = ({
  ship = null,
  name = '',
  claim = null,
  role = HOST_ROLE_SHIP,
  reconnectCredential = null,
}) => {
  if (hostRole(role) === HOST_ROLE_GM) {
    const body = { role: HOST_ROLE_GM, name };
    if (reconnectCredential != null) body.reconnect_credential = reconnectCredential;
    return hostFrame(HOST_FRAME_HELLO, body);
  }
  // `claim` (issue #1120) is the slot id a replacement machine is reclaiming; it
  // is absent for an ordinary join and only acted on after the freeze. Kept off
  // the body entirely when null, so a pre-#1120 lead sees an unchanged hello.
  return hostFrame(HOST_FRAME_HELLO, claim ? { ship, name, claim } : { ship, name });
};

export const welcomeFrame = (slotIdent, roster, {
  role = HOST_ROLE_SHIP,
  operatorId = null,
  reconnectCredential = null,
} = {}) => {
  if (hostRole(role) === HOST_ROLE_GM) {
    return hostFrame(HOST_FRAME_WELCOME, {
      slot: slotIdent,
      roster,
      role: HOST_ROLE_GM,
      operator_id: operatorId,
      reconnect_credential: reconnectCredential,
    });
  }
  return hostFrame(HOST_FRAME_WELCOME, { slot: slotIdent, roster });
};

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

/** One authenticated host's partial local start state; role is inferred by owner. */
export const startStateFrame = (state) => hostFrame(HOST_FRAME_START_STATE, state);

/** The owner's pure aggregate projection. */
export const startPolicyFrame = (policy) => hostFrame(HOST_FRAME_START_POLICY, { policy });

/** No claimed operator travels here; the receiving owner resolves the connection. */
export const startForceFrame = () => hostFrame(HOST_FRAME_START_FORCE, {});

/** Attributed force feedback, separate from the idempotent fleet-wide grant. */
export const forceResultFrame = (result) => hostFrame(HOST_FRAME_FORCE_RESULT, { result });

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
    ADMISSION_OPEN,
    ADMISSION_CLOSED,
    HOST_ROLE_SHIP,
    HOST_ROLE_GM,
    MAX_GM_OPERATORS,
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
    gmForPeer,
    gmById,
    slotById,
    admitHost,
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
    fleetPanelViewModel,
  };
}
