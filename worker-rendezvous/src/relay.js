/**
 * worker-rendezvous/src/relay.js — the secure-WebSocket game relay, as a pure
 * bounded mailbox hub (issue #1113).
 *
 * ## Why this is a separate file
 *
 * `src/registry.js` is the JOIN protocol: typed code lookup, admission,
 * presence, and the SDP/ICE relay that gets two browsers talking directly. This
 * module is the fallback for when that direct link cannot be built at all — a
 * public Wi-Fi that blocks UDP outright, a captive network that eats TURN, a
 * pair of CGNATs with no relay credential between them. In that case the same
 * rendezvous socket that carried the signalling carries the GAME, and a
 * signalling socket becomes a data path.
 *
 * That is a genuinely different resource profile, which is why it gets its own
 * module and its own authored bounds. A signalling peer exchanges two SDP blobs
 * and then costs the Durable Object nothing; a relayed crew member costs it
 * every command and every snapshot, for as long as the mission lasts.
 *
 * ## What it does NOT do — the load-bearing constraint
 *
 * It does not know what a game frame is. A relayed payload is an OPAQUE string
 * carrying exactly the bytes the DataChannel would have carried: the same
 * `ClientMessage`/`ServerMessage` JSON, the same in-band `JoinHandshake` /
 * `JoinAccepted` / `JoinRefused` compatibility handshake, the same `Identify`
 * gate on the host side, the same `localiseTree` ingress on the phone. There is
 * no second protocol here and nothing in this file may ever grow an opinion
 * about the contents of `payload` — that is what "the WebSocket fallback does
 * not fork the game protocol" means concretely.
 *
 * What it DOES keep is the delivery-class distinction, because that is a
 * transport property rather than a protocol one:
 *
 *   'reliable'  every frame is delivered, in order. May not drop. A queue that
 *               fills is a DEAD SESSION — closed, with a reason — because
 *               silently dropping a command would quietly break the guarantee
 *               the reliable DataChannel makes and the game is written against.
 *   'snapshot'  the lossy class. Drops OLDEST-first at the authored bound: a
 *               late snapshot is worthless and the next tick supersedes it, so
 *               dropping the stale one is the correct behaviour rather than a
 *               regrettable one. Drops are COUNTED and reported, so "the relay
 *               is shedding snapshots" is a diagnostics line rather than a
 *               mystery.
 *
 * ## Where the bound actually bites — say this precisely
 *
 * A bound nothing can reach is decoration, so be exact about which one this is.
 * A Durable Object is single-threaded, `registry.receive()` is synchronous, and
 * Cloudflare's WebSocket exposes no `bufferedAmount` — so this hub cannot
 * observe a socket's real send buffer and does not pretend to. What it holds is
 * a MEMORY ceiling on frames the adapter has not taken yet, and the adapter
 * stops taking them exactly when a target socket is not open (`setWritable`,
 * driven from `src/index.js`): a peer mid-close, or one whose socket the object
 * no longer holds. While a peer is unwritable its snapshot queue sheds and its
 * reliable queue does not, which is the asymmetry this whole module exists for.
 *
 * The OTHER half of the same rule — the one that meets real backpressure — is
 * the sender's, not the service's: `gui/rendezvous-relay.js` and the native
 * host's relay transport both queue against a socket's actual `bufferedAmount`,
 * which browsers and `tungstenite` do expose. This file is the service's share
 * of it, not the whole story.
 */

import { relayPayloadBytes } from '../../gui/rendezvous-protocol.js';

/**
 * Re-exported so the service's own bound and the client's local courtesy
 * refusal are measured by ONE function — see gui/rendezvous-protocol.js.
 */
export { relayPayloadBytes as payloadBytes };

/**
 * The two delivery classes a relayed frame may declare. Anything else is
 * refused rather than guessed at — a frame whose class the service cannot read
 * is one whose delivery guarantee it cannot honour.
 */
export const RELAY_RELIABLE = 'reliable';
export const RELAY_SNAPSHOT = 'snapshot';

/** True for a class name this hub will carry. */
export function isRelayClass(value) {
  return value === RELAY_RELIABLE || value === RELAY_SNAPSHOT;
}

/**
 * Authored bounds, with parse-time defaults so an older `join-codes.json` still
 * loads a worker built from this file (the same posture `createRegistry` takes
 * for the #1111 limits).
 *
 * @param {object} [limits] the `[limits]` table from assets/join/join-codes.toml
 */
export function relayLimits(limits = {}) {
  return {
    maxPeers: limits.max_relay_peers_per_record || 8,
    // The DataChannel's own SDP-negotiated ceiling, so a payload that crosses
    // the direct path crosses this one — see the authored table's comment.
    maxFrameBytes: limits.max_relay_frame_bytes || 262144,
    maxReliable: limits.max_relay_queue_reliable || 256,
    maxSnapshot: limits.max_relay_queue_snapshot || 32,
    // Not this hub's own bound — it is advertised to whoever is SENDING, whose
    // socket does report a backlog. See the module header.
    maxSendBufferBytes: limits.max_relay_send_buffer_bytes || 262144,
  };
}

/**
 * Build a relay hub.
 *
 * The hub owns nothing but mailboxes: who is attached, to which record, and
 * what is queued for them. It holds no socket (the adapter does) and no record
 * (the registry does), which is what lets the contract tests drive every rule
 * in here with plain objects.
 *
 * ## One mailbox per PAIR, not per connection
 *
 * A record's host is the target of every relaying joiner on it. Keying the
 * queue by the destination connection alone therefore put all N of them in one
 * box, and made both bounds record-wide by accident: while the host was
 * unwritable, one phone's reliable burst past `max_relay_queue_reliable` ended
 * the HOST's relay and left every other crew member answering `not-relaying`,
 * and one phone's snapshot burst evicted another's queued frames while the shed
 * count was attributed to whoever happened to send last. So a mailbox is keyed
 * by `(owner, from)`: the bound bites the pair that caused it, and the session
 * that ends is the one crew member on that pair.
 *
 * @param {object} [opts]
 * @param {object} [opts.limits] the authored `[limits]` table
 */
export function createRelayHub({ limits } = {}) {
  const bounds = relayLimits(limits);

  /**
   * Who is on the relay, independent of what is queued for them.
   *
   * @type {Map<string, { key: string, counted: boolean, writable: boolean }>}
   */
  const participants = new Map();

  /**
   * What is queued, per (owner, from) pair, created on first use.
   *
   * @type {Map<string, {
   *   owner: string,
   *   from: string,
   *   reliable: object[],
   *   snapshot: object[],
   *   dropped: number,
   *   overflowed: boolean,
   * }>}
   */
  const boxes = new Map();

  // A NUL separator, because a connection id can be anything the adapter mints
  // and two ids concatenated with a printable character could collide.
  const boxKey = (owner, from) => `${owner}\u0000${from}`;

  function boxFor(owner, from) {
    const key = boxKey(owner, from);
    let box = boxes.get(key);
    if (!box) {
      box = { owner, from, reliable: [], snapshot: [], dropped: 0, overflowed: false };
      boxes.set(key, box);
    }
    return box;
  }

  const boxesOwnedBy = (owner) => [...boxes.values()].filter((b) => b.owner === owner);

  /**
   * How many COUNTED peers are attached to one record key.
   *
   * A record's host is a participant here too — the many-phones-to-one-host
   * direction is exactly where a burst lands, and it needs the same bound — but
   * it is not one of the joiners the authored `max_relay_peers_per_record`
   * limits, so it attaches uncounted.
   */
  function countFor(key) {
    let n = 0;
    for (const p of participants.values()) if (p.key === key && p.counted) n += 1;
    return n;
  }

  return {
    limits: bounds,

    /** True once `attach` has taken this connection and before `detach`. */
    isAttached(peer) {
      return participants.has(peer);
    },

    /** The record key a peer is relaying through, or null. */
    keyFor(peer) {
      const p = participants.get(peer);
      return p ? p.key : null;
    },

    /**
     * Every COUNTED peer attached to one record, in attachment order — the
     * joiners, not the record host's own uncounted attachment.
     */
    peersFor(key) {
      return [...participants.entries()]
        .filter(([, p]) => p.key === key && p.counted)
        .map(([id]) => id);
    },

    /**
     * Attach `peer` to `key`'s relay.
     *
     * Returns `{ ok: true }` or `{ ok: false, reason }`. `relay-full` is its own
     * reason rather than being folded into the join path's `admission-closed`:
     * a crew list that is full and a relay that is full are different problems
     * with different remedies, and the phone's diagnostics say which.
     *
     * `counted: false` is how a record's HOST attaches without spending one of
     * the joiner slots — see `countFor`.
     */
    attach(peer, key, { counted = true } = {}) {
      const existing = participants.get(peer);
      if (existing) {
        return existing.key === key ? { ok: true } : { ok: false, reason: 'already-relaying' };
      }
      if (counted && countFor(key) >= bounds.maxPeers) return { ok: false, reason: 'relay-full' };
      participants.set(peer, {
        key,
        counted,
        // Writable until the adapter says otherwise. A socket the object holds
        // and has accepted is open, so this is the ordinary state; `false` is
        // a peer mid-close or one whose socket has already gone.
        writable: true,
      });
      return { ok: true };
    },

    /**
     * Report whether the adapter can currently put bytes on `peer`'s socket.
     *
     * This is the ONLY backpressure signal a Durable Object has — see the
     * module header. While a peer is unwritable its mailboxes hold, which is
     * when the authored queue depths do their work.
     */
    setWritable(peer, writable) {
      const p = participants.get(peer);
      if (p) p.writable = !!writable;
    },

    /** True when `drain` will hand this peer's frames over. */
    isWritable(peer) {
      const p = participants.get(peer);
      return !p || p.writable;
    },

    /**
     * Detach `peer`, discarding everything queued FOR it and everything queued
     * BY it. Both halves matter: a pair has two ends, and leaving the other
     * end's box behind would leak a queue nothing will ever drain.
     */
    detach(peer) {
      const had = participants.delete(peer);
      for (const [key, box] of [...boxes]) {
        if (box.owner === peer || box.from === peer) boxes.delete(key);
      }
      return had;
    },

    /**
     * Queue one frame from `from`, for `owner`.
     *
     * @returns {{ok: true, dropped: number, totalDropped: number}
     *   | {ok: false, reason: string}}
     *   `dropped` is how many SNAPSHOT frames THIS enqueue displaced;
     *   `totalDropped` is the pair's running total, which is the number worth
     *   putting in front of a person — a per-enqueue delta is 1 essentially
     *   always, and a readout showing "1" forever while hundreds are lost is
     *   worse than no readout.
     */
    enqueue(owner, from, cls, frame, payload) {
      if (!participants.has(owner)) return { ok: false, reason: 'not-relaying' };
      if (!isRelayClass(cls)) return { ok: false, reason: 'malformed' };
      if (relayPayloadBytes(payload) > bounds.maxFrameBytes) {
        return { ok: false, reason: 'relay-too-large' };
      }
      const box = boxFor(owner, from);
      if (cls === RELAY_SNAPSHOT) {
        box.snapshot.push(frame);
        let dropped = 0;
        while (box.snapshot.length > bounds.maxSnapshot) {
          box.snapshot.shift();
          dropped += 1;
        }
        box.dropped += dropped;
        return { ok: true, dropped, totalDropped: box.dropped };
      }
      // The reliable class may not shed anything, so a full queue is the end of
      // that pair's session rather than a quiet loss. Everything queued is
      // discarded with it, and this is the honest reading of why: a queue can
      // only exceed its bound while the owner is UNWRITABLE (the registry
      // flushes after every enqueue), and `drain` hands nothing over for an
      // unwritable peer — so there is no "whatever fitted is delivered first".
      // The frames go, the session ends, and it ends with a reason both ends
      // can render rather than becoming a quietly lossy reliable channel.
      box.reliable.push(frame);
      if (box.reliable.length > bounds.maxReliable) box.overflowed = true;
      return { ok: true, dropped: 0, totalDropped: box.dropped };
    },

    /**
     * True once any queue held FOR `owner` has passed its reliable bound.
     */
    hasOverflowed(owner) {
      return boxesOwnedBy(owner).some((b) => b.overflowed);
    },

    /**
     * Which SENDERS overflowed a queue held for `owner`. The caller ends the
     * session of the crew member on each of those pairs — see `enqueue`.
     */
    overflowedSources(owner) {
      return boxesOwnedBy(owner)
        .filter((b) => b.overflowed)
        .map((b) => b.from);
    },

    /**
     * Take everything queued for `owner`, reliable class first across every
     * sender — or nothing at all while that peer is unwritable, which is what
     * makes the queue a queue.
     *
     * Reliable before snapshot on purpose: within one drain the queues are
     * concurrent, and if a burst is being shed anyway the commands are the half
     * that must not wait behind snapshots that are about to be superseded.
     */
    drain(owner) {
      const p = participants.get(owner);
      if (!p || !p.writable) return [];
      const held = boxesOwnedBy(owner);
      const frames = [
        ...held.flatMap((b) => b.reliable),
        ...held.flatMap((b) => b.snapshot),
      ];
      for (const b of held) {
        b.reliable = [];
        b.snapshot = [];
      }
      return frames;
    },

    /**
     * A peer's counters, for diagnostics: how many snapshot frames the mailboxes
     * held for it have shed over their whole life, and whether any reliable
     * queue overflowed.
     */
    stats(peer) {
      if (!participants.has(peer)) return null;
      const held = boxesOwnedBy(peer);
      return {
        dropped: held.reduce((n, b) => n + b.dropped, 0),
        overflowed: held.some((b) => b.overflowed),
      };
    },
  };
}
