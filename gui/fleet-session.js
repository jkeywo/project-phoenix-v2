/**
 * gui/fleet-session.js — the two ends of a fleet, wired onto the Phoenix
 * transport (issue #1114).
 *
 * `gui/host-mesh.js` decides WHAT a fleet is and what each frame means; this
 * module is the part that owns a socket. The split is the same one
 * `lobby/handler.rs` and `lobby/server.rs` keep on the Rust side, and for the
 * same reason: the awkward decisions (what a `hello` is answered with once the
 * mission has started, what happens to a slot when its host drops) are testable
 * without a transport, and what is left here is plumbing.
 *
 * ## Two halves
 *
 *   createFleetOwner   the host that OPENED the fleet. It registers a SECOND
 *                      host record — in the `server` namespace, alongside its
 *                      own crew record — is issued the one privileged fleet
 *                      code, and holds the roster.
 *   createFleetMember  a host that typed that code. It joins the owner's record
 *                      exactly as a phone joins a ship's, announces itself and
 *                      then renders whatever roster the owner sends.
 *
 * ## What this is NOT
 *
 * It is not a crew connection, and the separation is load-bearing rather than
 * stylistic. A fleet member never sends `Identify`, is never given a session
 * token, never appears in `peerTokens`/`tokenConns`, and nothing it says
 * reaches `wasm_receive_message`. Each crew star stays attached to its own
 * host because the fleet link is a different socket, in a different namespace,
 * carrying a different vocabulary — not because anything downstream filters it.
 *
 * ## Two vocabularies on one link (issue #1116)
 *
 * Since the fleet became something that RUNS a mission, this link carries two
 * kinds of frame and routes them to two different places:
 *
 *   lobby frames        (hello / welcome / refused / slot / roster / admission)
 *                       are decided here, by the model in `gui/host-mesh.js`.
 *   simulation frames   (tick / digest) are handed straight to the wasm
 *                       boundary through `onSimulationFrame`, and their bodies
 *                       are never read on this side.
 *
 * `isSimulationFrame` is the whole of the rule, and it matters in both
 * directions: a lobby frame that reached the simulation would be a roster edit
 * nobody admitted, and a tick frame that reached the fleet model would be
 * silently dropped — the fleet would then stall on the peer that sent it and
 * nobody would be able to say why.
 *
 * The simulation's frames are opaque here on purpose. JavaScript that could
 * BUILD one could build a command no host's authority gate ever accepted; Rust
 * mints them (`core::codec::encode_mesh_frame`), and this module is the wire.
 *
 * ## Where the roster is shown, and where it will be shown
 *
 * Today the synced roster reaches each host's OWN operator surface: the
 * `#fleet-panel` on its viewscreen and the Fleet section of its settings cog.
 * That is the whole of #1114.
 *
 * Reaching a PHONE is a different route and it is worth writing down, because
 * the obvious one is wrong. A fleet frame must never be forwarded to a crew
 * client — that would put another ship's host protocol on a console's wire and
 * make "each crew star belongs to one host" a forwarding rule rather than a
 * fact. The roster reaches a phone the way everything else on that phone does:
 * its OWN host projects it, as an ordinary `ServerMessage` broadcast to that
 * ship's crew alone, fed in from this page through the same wasm boundary the
 * pre-Bevy scenario catalogue already crosses. So a crew member sees the fleet
 * their ship is in, described by their own ship's host, and never a byte from
 * another host's link. That message belongs with #1116, which is when a crew
 * has something to do about it.
 */

import { NAMESPACE_SERVER } from './join-code.js';
import { createRendezvousHost, createRendezvousJoiner } from './rendezvous-transport.js';
import {
  ADMISSION_CLOSED,
  ADMISSION_OPEN,
  HOST_FRAME_ADMISSION,
  HOST_FRAME_HELLO,
  HOST_FRAME_REFUSED,
  HOST_FRAME_ROSTER,
  HOST_FRAME_SLOT,
  HOST_FRAME_WELCOME,
  admissionFrame,
  admitHost,
  asHostFrame,
  decodeHostFrame,
  dropHost,
  encodeHostFrame,
  freezeFleet,
  helloFrame,
  hostSlotOrdinal,
  isSimulationFrame,
  openFleet,
  refusedFrame,
  rosterFrame,
  rosterOf,
  setAdmission,
  slotFrame,
  slotForPeer,
  updateSlot,
  welcomeFrame,
} from './host-mesh.js';

/**
 * The verdict a fleet uses when nobody supplied a real one.
 *
 * Deliberately the OPPOSITE of the crew path's default. `createRendezvousHost`
 * admits when it cannot reach `wasm_check_client_stamp`, because a host that
 * cannot ask cannot refuse and locking out every phone over an unreachable
 * export is the worse failure. A fleet has no such asymmetry: admitting a ship
 * host whose build was never checked is how two authoritative simulations end
 * up quietly running different content, which is the exact failure
 * `delivery::check_host_stamp` exists to prevent. So the safe default here is
 * to refuse.
 */
const REFUSE_UNCHECKED = () => ({
  ok: false,
  code: 'client-stamp-missing',
  detail: 'this host could not check the joining build',
});

// ── Owner ───────────────────────────────────────────────────────────────────

/**
 * Open a fleet on this host and admit ship hosts to it.
 *
 * @param {object} opts
 * @param {string} opts.base rendezvous service base URL
 * @param {number} opts.maxSlots authored fleet capacity (`[limits]
 *   max_fleet_hosts` in assets/join/join-codes.toml)
 * @param {number} [opts.maxNameLength] `[limits] max_slot_name_length`
 * @param {number} [opts.maxShipPathLength] `[limits] max_slot_ship_path_length`
 * @param {object[]} [opts.iceServers]
 * @param {(stamp:string|null)=>{ok:boolean,code?:string,detail?:string}} [opts.checkStamp]
 *   the authoritative host-to-host verdict — `wasm_check_host_stamp`.
 * @param {{template_path: string|null, name?: string}|null} [opts.ship]
 * @param {string} [opts.name]
 * @param {(code:object)=>void} [opts.onCode] the issued fleet code; fires again
 *   with a NEW one if the service was lost and this host re-registered.
 * @param {(roster:object)=>void} [opts.onRoster] every change, owner included.
 * @param {(raw:string)=>void} [opts.onSimulationFrame] a `tick` or `digest`
 *   frame from a member, still encoded — hand it to `wasm_receive_mesh_frame`.
 * @param {(reason:string, detail?:string)=>void} [opts.onError]
 * @param {(msg:string)=>void} [opts.onLog]
 */
export function createFleetOwner(opts) {
  const {
    base,
    maxSlots,
    maxNameLength,
    maxShipPathLength,
    iceServers = [],
    checkStamp = REFUSE_UNCHECKED,
    ship = null,
    name = '',
    onCode = () => {},
    onRoster = () => {},
    onSimulationFrame = () => {},
    onHostLost = () => {},
    onError = () => {},
    onLog = () => {},
    factories,
  } = opts;

  let fleet = openFleet({ ship, name, maxSlots, maxNameLength, maxShipPathLength });
  /** rendezvous peer id → the admitted connection adapter. */
  const links = new Map();
  let code = null;

  const publish = () => {
    const roster = rosterOf(fleet);
    const json = encodeHostFrame(rosterFrame(fleet));
    for (const conn of links.values()) conn.send(json);
    onRoster(roster);
  };

  function onHello(conn, body) {
    const verdict = admitHost(fleet, {
      peer: conn.peer,
      ship: body.ship || null,
      name: body.name || '',
    });
    if (!verdict.ok) {
      onLog(`[fleet] refusing ${String(conn.peer).slice(0, 8)}…: ${verdict.reason}`);
      conn.send(encodeHostFrame(refusedFrame(verdict.reason, { of: HOST_FRAME_HELLO })));
      // Told, then severed. A host left holding an open channel it may never
      // use again would sit rendering an empty fleet panel with no idea why —
      // the same reasoning behind the crew path's refuse-then-close.
      setTimeout(() => conn.close(), 250);
      return;
    }
    fleet = verdict.fleet;
    links.set(conn.peer, conn);
    conn.send(encodeHostFrame(welcomeFrame(verdict.slot.id, rosterOf(fleet))));
    publish();
  }

  function onSlot(conn, body) {
    const slot = slotForPeer(fleet, conn.peer);
    if (!slot) return;
    const result = updateSlot(fleet, slot.id, body);
    if (!result.ok) {
      // The freeze, answered to a member that had not heard about it yet.
      //
      // Subject `slot`, and that is the whole point of the field: this host is
      // in the fleet and stays in it. An unqualified refusal would reach the
      // member's terminal `onError` and tear down a link that is doing nothing
      // wrong — the answer is about the PATCH, not about the membership.
      conn.send(encodeHostFrame(refusedFrame(result.reason, { of: HOST_FRAME_SLOT })));
      return;
    }
    fleet = result.fleet;
    publish();
  }

  // Declared before the transport because its own callbacks reach back for it;
  // they only ever run off a socket event, but a `const` in a temporal dead
  // zone is a footgun aimed at whoever next makes one of those synchronous.
  let host = null;
  host = createRendezvousHost({
    base,
    namespace: NAMESPACE_SERVER,
    iceServers,
    factories,
    checkStamp,
    onCode: (issued) => {
      // The service answers `host-admission` with a `hosted` frame too, so this
      // fires for an admission ACK as well as for a registration. Same letters
      // means nothing was issued: repainting the panel would be noise, and
      // re-asserting admission below would be an infinite exchange with the
      // service.
      const reissued = !code || code.full !== issued.full;
      code = issued;
      if (!reissued) return;
      onLog(`[fleet] issued fleet code ${issued.suffix}`);
      onCode(issued);
      // A replacement registration starts in the service's default state, so
      // whatever this fleet last told the SERVICE has to be said again.
      //
      // Which is not the model's `admission`. `freezeFleet` sets that to
      // `closed` — the roster can create no more slots — while `freeze()`
      // deliberately tells the service `open`, because `p2p-fixed-host-slot-
      // recovery` makes the server code a recovery capability that must stay
      // RESOLVABLE across mission start so a later claim can reach this host to
      // be judged. Replaying the model flag here would re-close the door the
      // freeze opened on purpose, and a reconnect after mission start would
      // silently become the permanent lockout the freeze exists to prevent.
      // There is one rule for what state the service record should be in, and
      // this is it, stated the same way in both places.
      if (fleet.frozen) host.setAdmission(ADMISSION_OPEN);
      else if (fleet.admission === ADMISSION_CLOSED) host.setAdmission(ADMISSION_CLOSED);
    },
    onConnection: (conn) => {
      conn.on('data', (raw) => {
        const frame = decodeHostFrame(raw);
        if (!frame) return;
        if (isSimulationFrame(frame)) {
          // This host's own simulation needs it…
          onSimulationFrame(raw);
          // …and so does every OTHER member. The transport is a star with the
          // fleet lead at the centre (#1114), so a member's tick frame reaches
          // its siblings only if the lead passes it on. Relayed VERBATIM and
          // unread: it is the sender's statement about its own crew, and the
          // lead has no more right to edit it than to invent it. A fleet of two
          // has no siblings and this loop does nothing.
          for (const [peer, other] of links) {
            if (peer !== conn.peer) other.send(raw);
          }
          return;
        }
        if (frame.t === HOST_FRAME_HELLO) onHello(conn, frame.d);
        else if (frame.t === HOST_FRAME_SLOT) onSlot(conn, frame.d);
        // Everything else is owner-to-member and is noise arriving upstream.
      });
      conn.on('close', () => {
        if (!links.delete(conn.peer)) return;
        // Before the mission starts, a host closing is just a lobby slot going
        // dark. Once frozen it is a HOST LOSS (issue #1119): the simulation must
        // flip that ship to Backfill at an agreed tick, so the slot is resolved
        // and reported to the simulation BEFORE `dropHost` clears its peer. The
        // simulation mints the tick-stamped host-loss frame from there and this
        // page relays it to the rest of the fleet like any other simulation
        // frame — so a member learns of a sibling's loss without seeing its
        // socket.
        if (fleet.frozen) {
          const slot = slotForPeer(fleet, conn.peer);
          const ordinal = slot ? hostSlotOrdinal(slot.id) : null;
          if (ordinal != null) {
            onLog(`[fleet] ${slot.id} left mid-mission — backfilling its ship`);
            onHostLost(ordinal);
          }
        }
        fleet = dropHost(fleet, conn.peer);
        publish();
      });
    },
    onError,
    onLog,
  });

  publish();

  return {
    get code() { return code; },
    get isOwner() { return true; },
    get slot() { return fleet.owner; },
    roster: () => rosterOf(fleet),

    /**
     * The operator's lever. Both halves matter: the SERVICE stops resolving
     * the code for a new joiner, and the fleet stops answering a `hello` from
     * one that resolved it a moment before the close. Neither touches a host
     * already in the roster.
     */
    setAdmission(state) {
      // Inert once frozen, in BOTH directions. Reopening cannot un-launch a
      // mission, and closing would shut the service door on the recovery path
      // `freeze()` below deliberately leaves open.
      if (fleet.frozen) return;
      const next = state === ADMISSION_CLOSED ? ADMISSION_CLOSED : ADMISSION_OPEN;
      fleet = setAdmission(fleet, next);
      host.setAdmission(next);
      for (const conn of links.values()) conn.send(encodeHostFrame(admissionFrame(next)));
      publish();
    },

    /**
     * Mission start: the slot roster and every selected loadout freeze.
     *
     * The service-side gate is RELEASED here rather than tightened, which looks
     * backwards and is not. `p2p-fixed-host-slot-recovery` makes the server
     * code a privileged RECOVERY capability that survives mission start —
     * "closing new-host admission does not invalidate it for a known
     * disconnected slot" — so an entry after the freeze has to REACH this host
     * to be judged. A record the service refuses at lookup could never carry
     * that claim, and the operator's pre-start closure would silently become a
     * permanent lockout of the replacement machine.
     *
     * What it is judged as is #1114's honest limit: `admitHost` answers
     * `recovery-only` for every post-freeze `hello`, because performing the
     * recovery — restoring the slot from the shared snapshot and command
     * history — is #1120's. The distinct reason is the seam that work lands in.
     */
    freeze() {
      if (fleet.frozen) return;
      fleet = freezeFleet(fleet);
      host.setAdmission(ADMISSION_OPEN);
      publish();
    },

    /**
     * Explicit rotation of the fleet's OWN join code (issue #1115 AC2/AC3) —
     * a pure passthrough onto the underlying host's `rotate()`. Deliberately
     * NOT gated on `fleet.frozen` here: `frozen` governs whether the SLOT
     * ROSTER may change, which has nothing to do with which letters admit a
     * new ship host to it, and the freeze latch never resets even once the
     * mission reaches GameOver (#1116 is what would make it). The
     * mission-phase gate this AC actually wants — rotatable in Lobby/GameOver,
     * refused in Loading/InProgress — is the CALLER's (server.html's
     * `hostCanRotateCodes`, built on gui/phase-toggle.js's `codesRotatable`),
     * for the same reason `host.rotate()` itself carries no such gate: the
     * registry, and everything under it, has no notion of GamePhase at all.
     */
    rotate() {
      host.rotate();
    },

    /**
     * Say something to the whole fleet, verbatim (issue #1116).
     *
     * What the simulation produced through `wasm_take_mesh_frames`, sent to
     * every admitted member. Deliberately takes the ENCODED frame rather than a
     * body to wrap: the envelope came from the same encoder the receiving
     * simulation decodes with, and a second wrapping on this side would be a
     * second place for the two to disagree about the shape.
     */
    broadcast(raw) {
      for (const conn of links.values()) conn.send(raw);
    },

    /** The owner's own ship/readiness, which follow the same freeze. */
    update(patch) {
      const result = updateSlot(fleet, fleet.owner, patch);
      if (!result.ok) return false;
      fleet = result.fleet;
      publish();
      return true;
    },

    close() {
      for (const conn of links.values()) conn.close();
      links.clear();
      host.close();
    },
  };
}

// ── Member ──────────────────────────────────────────────────────────────────

/**
 * Join the fleet behind a typed server code.
 *
 * @param {object} opts
 * @param {string} opts.base
 * @param {string} opts.code the five letters, or a pasted full code
 * @param {object} opts.data the authored join table
 * @param {string|null} opts.stamp this host's own `p/id/epoch` field
 * @param {{template_path: string|null, name?: string}|null} [opts.ship]
 * @param {string} [opts.name]
 * @param {(roster:object)=>void} [opts.onRoster]
 * @param {(slotId:string, roster:object)=>void} [opts.onWelcome]
 * @param {(reason:string, detail?:string)=>void} [opts.onError] a TERMINAL
 *   refusal — of this host's admission, or from the service — as a machine
 *   reason. The link is over by the time this fires.
 * @param {(reason:string, detail?:string)=>void} [opts.onRefusedSlot] the fleet
 *   declined a change this host asked for about its OWN slot. Not terminal:
 *   this host is still in the fleet, and the roster that follows is the truth.
 * @param {(raw:string)=>void} [opts.onSimulationFrame] a `tick` or `digest`
 *   frame from another host, still encoded — hand it to
 *   `wasm_receive_mesh_frame`.
 * @param {(status:string)=>void} [opts.onStatus]
 * @param {(msg:string)=>void} [opts.onLog]
 */
export function createFleetMember(opts) {
  const {
    base,
    code,
    data,
    stamp = null,
    iceServers = [],
    ship = null,
    name = '',
    onRoster = () => {},
    onWelcome = () => {},
    onSimulationFrame = () => {},
    onError = () => {},
    onRefusedSlot = () => {},
    onStatus = () => {},
    onLog = () => {},
    factories,
  } = opts;

  let mine = null;
  let roster = null;
  let announced = { ship, name };
  // Declared before the joiner because its own callbacks reach back for it.
  // They only ever run after an async socket event, so the assignment below has
  // always happened by then — but a `const` in a temporal dead zone is a
  // footgun aimed at whoever next makes one of these paths synchronous.
  let joiner = null;

  joiner = createRendezvousJoiner({
    base,
    data,
    code,
    namespace: NAMESPACE_SERVER,
    stamp,
    iceServers,
    factories,
    // A ship host is not a crew member: no Identify, and no string ids to
    // resolve in a vocabulary that carries none.
    localise: false,
    onAccepted: () => {
      // No stamp travels in the `hello`: the build check already happened on
      // the transport plane, over this very channel, and a second copy of it
      // in the fleet vocabulary would be an unvalidated peer claim sitting
      // where a future reader looks first.
      joiner.sendFrame(encodeHostFrame(helloFrame({
        ship: announced.ship,
        name: announced.name,
      })));
    },
    onData: (frame) => {
      const decoded = asHostFrame(frame);
      if (!decoded) return;
      if (isSimulationFrame(decoded)) {
        // Re-encoded rather than passed through: the transport hands this
        // callback an already-PARSED value, and what the wasm boundary takes is
        // text. Encoding the decoded frame (not the raw input) is what makes
        // the envelope that reaches the simulation the one this module
        // recognised, rather than whatever arrived.
        onSimulationFrame(encodeHostFrame(decoded));
        return;
      }
      if (decoded.t === HOST_FRAME_WELCOME) {
        mine = decoded.d.slot || null;
        roster = decoded.d.roster || null;
        onLog(`[fleet] admitted as ${mine}`);
        onWelcome(mine, roster);
        onRoster(roster);
        return;
      }
      if (decoded.t === HOST_FRAME_ROSTER) {
        roster = decoded.d.roster || null;
        onRoster(roster);
        return;
      }
      if (decoded.t === HOST_FRAME_ADMISSION) {
        // Advisory: the roster that follows carries the same state. Kept as
        // its own frame because a member that is mid-roster-sync still needs
        // to be able to say "the fleet just closed" without waiting.
        if (roster) {
          roster = { ...roster, admission: decoded.d.state };
          onRoster(roster);
        }
        return;
      }
      if (decoded.t === HOST_FRAME_REFUSED) {
        const code = decoded.d.code;
        const detail = decoded.d.detail || '';
        // WHICH request was refused decides whether this link is over.
        //
        // A refusal of the `slot` patch this host sent about ITSELF says
        // nothing about its membership — the commonest case is a loadout
        // change that raced the freeze — so it is reported and the roster the
        // owner sends next is the truth. Only a refusal of the admission
        // itself is terminal, and a refusal that arrives before this host was
        // ever seated is one of those whatever it claims to answer.
        const aboutTheSlot = decoded.d.of === HOST_FRAME_SLOT && !!mine;
        if (aboutTheSlot) {
          onLog(`[fleet] the fleet refused that change: ${code}`);
          onRefusedSlot(code, detail);
          return;
        }
        onLog(`[fleet] the fleet refused this host: ${code}`);
        onError(code, detail);
      }
    },
    onStatus,
    onError,
    onLog,
  });

  return {
    get isOwner() { return false; },
    get slot() { return mine; },
    get code() { return joiner.failed ? null : { suffix: joiner.suffix, full: joiner.full }; },
    roster: () => roster,

    /**
     * Say something to the fleet, verbatim (issue #1116).
     *
     * A member speaks to the lead, which relays to the rest — the star this
     * transport is. See the lead's own `broadcast` for why the frame is sent as
     * it came rather than re-wrapped.
     */
    broadcast(raw) {
      joiner.sendFrame(raw);
    },

    /** Announce this host's own ship or readiness to the fleet owner. */
    update(patch) {
      announced = { ...announced, ...patch };
      joiner.sendFrame(encodeHostFrame(slotFrame(patch)));
    },

    close() {
      joiner.close();
    },
  };
}

// Expose for the classic-script bootstrap in server.html.
if (typeof window !== 'undefined') {
  window.fleetSession = { createFleetOwner, createFleetMember };
}
