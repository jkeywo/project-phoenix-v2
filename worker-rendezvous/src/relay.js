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
    maxFrameBytes: limits.max_relay_frame_bytes || 65536,
    maxReliable: limits.max_relay_queue_reliable || 256,
    maxSnapshot: limits.max_relay_queue_snapshot || 32,
  };
}

/**
 * Byte length of a relayed payload, measured the way the wire measures it.
 *
 * `String.length` counts UTF-16 code units, which under-counts every non-ASCII
 * character a display name or a comms line may carry — so a bound written
 * against it would be a different, larger bound than the one authored. Workers,
 * browsers and Node all have `TextEncoder`; the `Blob` arm is a defence for an
 * exotic host rather than a path anything shipped takes.
 */
export function payloadBytes(payload) {
  if (typeof payload !== 'string') return Infinity;
  if (typeof TextEncoder !== 'undefined') return new TextEncoder().encode(payload).length;
  return payload.length;
}

/**
 * Build a relay hub.
 *
 * The hub owns nothing but mailboxes: who is attached, to which record, and
 * what is queued for them. It holds no socket (the adapter does) and no record
 * (the registry does), which is what lets the contract tests drive every rule
 * in here with plain objects.
 *
 * @param {object} [opts]
 * @param {object} [opts.limits] the authored `[limits]` table
 */
export function createRelayHub({ limits } = {}) {
  const bounds = relayLimits(limits);

  /**
   * @type {Map<string, {
   *   key: string,
   *   reliable: object[],
   *   snapshot: object[],
   *   dropped: number,
   *   overflowed: boolean,
   * }>} peer connection id → mailbox
   */
  const boxes = new Map();

  /**
   * How many COUNTED peers are attached to one record key.
   *
   * A record's host holds a mailbox here too — the many-phones-to-one-host
   * direction is exactly where a burst lands, and it needs the same bound — but
   * it is not one of the joiners the authored `max_relay_peers_per_record`
   * limits, so it attaches uncounted.
   */
  function countFor(key) {
    let n = 0;
    for (const box of boxes.values()) if (box.key === key && box.counted) n += 1;
    return n;
  }

  return {
    limits: bounds,

    /** True once `attach` has taken this connection and before `detach`. */
    isAttached(peer) {
      return boxes.has(peer);
    },

    /** The record key a peer is relaying through, or null. */
    keyFor(peer) {
      const box = boxes.get(peer);
      return box ? box.key : null;
    },

    /**
     * Every COUNTED peer attached to one record, in attachment order — the
     * joiners, not the host's own uncounted mailbox.
     */
    peersFor(key) {
      return [...boxes.entries()]
        .filter(([, b]) => b.key === key && b.counted)
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
     * `counted: false` is how a record's HOST takes a mailbox without spending
     * one of the joiner slots — see `countFor`.
     */
    attach(peer, key, { counted = true } = {}) {
      const existing = boxes.get(peer);
      if (existing) {
        return existing.key === key ? { ok: true } : { ok: false, reason: 'already-relaying' };
      }
      if (counted && countFor(key) >= bounds.maxPeers) return { ok: false, reason: 'relay-full' };
      boxes.set(peer, {
        key,
        counted,
        reliable: [],
        snapshot: [],
        dropped: 0,
        overflowed: false,
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
     * module header. While a peer is unwritable its mailbox holds, which is
     * when the authored queue depths do their work.
     */
    setWritable(peer, writable) {
      const box = boxes.get(peer);
      if (box) box.writable = !!writable;
    },

    /** True when `drain` will hand this peer's frames over. */
    isWritable(peer) {
      const box = boxes.get(peer);
      return !box || box.writable;
    },

    /** Detach `peer`, discarding anything still queued for it. */
    detach(peer) {
      return boxes.delete(peer);
    },

    /**
     * Queue one frame for `peer`.
     *
     * @returns {{ok: true, dropped: number} | {ok: false, reason: string}}
     *   `dropped` is how many SNAPSHOT frames this enqueue displaced — the
     *   number the sender is told about so "the relay is shedding snapshots"
     *   can reach a diagnostics readout instead of being invisible.
     */
    enqueue(peer, cls, frame, payload) {
      const box = boxes.get(peer);
      if (!box) return { ok: false, reason: 'not-relaying' };
      if (!isRelayClass(cls)) return { ok: false, reason: 'malformed' };
      if (payloadBytes(payload) > bounds.maxFrameBytes) {
        return { ok: false, reason: 'relay-too-large' };
      }
      if (cls === RELAY_SNAPSHOT) {
        box.snapshot.push(frame);
        let dropped = 0;
        while (box.snapshot.length > bounds.maxSnapshot) {
          box.snapshot.shift();
          dropped += 1;
        }
        box.dropped += dropped;
        return { ok: true, dropped };
      }
      // The reliable class may not shed anything, so a full queue is the end of
      // the session rather than a quiet loss. The frame is still queued: the
      // adapter drains before it acts on the overflow, so whatever fitted is
      // delivered and the peer is then closed with a reason it can render.
      box.reliable.push(frame);
      if (box.reliable.length > bounds.maxReliable) box.overflowed = true;
      return { ok: true, dropped: 0 };
    },

    /**
     * True once a peer's reliable queue has passed its bound. The caller ends
     * that session — see `enqueue`.
     */
    hasOverflowed(peer) {
      const box = boxes.get(peer);
      return !!(box && box.overflowed);
    },

    /**
     * Take everything queued for `peer`, reliable class first — or nothing at
     * all while that peer is unwritable, which is what makes the queue a queue.
     *
     * Reliable before snapshot on purpose: within one drain the two queues are
     * concurrent, and if a burst is being shed anyway the commands are the half
     * that must not wait behind snapshots that are about to be superseded.
     */
    drain(peer) {
      const box = boxes.get(peer);
      if (!box || !box.writable) return [];
      const frames = [...box.reliable, ...box.snapshot];
      box.reliable = [];
      box.snapshot = [];
      return frames;
    },

    /**
     * A peer's counters, for diagnostics: how many snapshot frames its mailbox
     * has shed over its whole life, and whether its reliable queue overflowed.
     */
    stats(peer) {
      const box = boxes.get(peer);
      if (!box) return null;
      return { dropped: box.dropped, overflowed: box.overflowed };
    },
  };
}
