/**
 * gui/rendezvous-relay.js — the game's own frames over the rendezvous socket
 * (issue #1113).
 *
 * ## What this is for
 *
 * Some networks will not build a direct WebRTC link at any price: a public
 * Wi-Fi that blocks UDP and eats TURN, a captive portal that permits nothing
 * but HTTPS, a pair of CGNATs with no relay credential between them. Before
 * #1112 that meant a degraded connection; since PeerJS was retired it means no
 * connection at all. So when the direct ladder is spent, the rendezvous socket
 * — a plain outbound `wss:` connection, which those networks do allow, because
 * the phone already used it to look the code up — carries the game instead.
 *
 * ## The one rule this module exists to keep
 *
 * **It does not fork the game protocol.** A relayed payload is the SAME string
 * the DataChannel would have carried: the same `ClientMessage`/`ServerMessage`
 * JSON, the same in-band `JoinHandshake`/`JoinAccepted`/`JoinRefused`
 * compatibility handshake, the same `Identify` gate on the host, the same
 * `localiseTree` ingress on the phone. Nothing downstream of this file can tell
 * which transport it is on, and nothing in it may ever grow an opinion about
 * what is inside a payload.
 *
 * The way that rule is *enforced* rather than merely intended: this module
 * hands back objects shaped like `RTCDataChannel` — `label`, `readyState`,
 * `send`, `close`, `onopen`/`onmessage`/`onclose`/`onerror` — and
 * gui/rendezvous-transport.js wires them through the same `connectionAdapter`,
 * the same admission gate and the same delivery-class routing it wires a real
 * channel through. There is one code path, and the relay is a different pair of
 * channels on it, not a second one.
 *
 * ## The lossy half stays lossy
 *
 * A WebSocket is reliable and ordered end to end, so a naive relay would
 * silently upgrade the snapshot class into a reliable one — and head-of-line
 * blocking on a phone's radio is precisely what that class exists to avoid. So
 * the snapshot channel sheds instead: when the socket's own `bufferedAmount`
 * has passed the authored ceiling, a snapshot frame is DROPPED rather than
 * queued behind the backlog, because a late snapshot is worthless and the next
 * tick supersedes it. Reliable frames are never dropped here — they go to the
 * socket, which buffers them — because a silently dropped command would break
 * the guarantee the game is written against.
 *
 * `bufferedAmount` is the real backpressure signal on this side of the wire,
 * which is why the shedding rule lives here rather than in the service: a
 * Durable Object's WebSocket does not expose one (see
 * worker-rendezvous/src/relay.js).
 */

import { relayPayloadBytes } from './rendezvous-protocol.js';

/** Label of the reliable relayed channel — the same label the DataChannel uses. */
export const RELAY_RELIABLE_LABEL = 'reliable';
/** Label of the lossy relayed channel. */
export const RELAY_SNAPSHOT_LABEL = 'snapshot';

/**
 * Fallback ceilings, used only until the service's `relay-ready` frame arrives
 * with the authored ones. Deliberately conservative: a joiner that guessed
 * generously and then met a stricter service would be cut off mid-mission
 * rather than shedding a snapshot.
 */
export const RELAY_LIMIT_DEFAULTS = {
  maxFrameBytes: 65536,
  maxSendBufferBytes: 262144,
};

/**
 * Read the limits the service advertised on `relay-ready` into this module's
 * shape, falling back per field. The wire names are the service's snake_case
 * ones; nothing else in the client should have to know them.
 */
export function relayLimitsFromFrame(limits) {
  const l = limits || {};
  return {
    maxFrameBytes: l.max_frame_bytes || RELAY_LIMIT_DEFAULTS.maxFrameBytes,
    maxSendBufferBytes: l.max_send_buffer_bytes || RELAY_LIMIT_DEFAULTS.maxSendBufferBytes,
  };
}

/**
 * Build a pair of DataChannel-shaped objects backed by a rendezvous socket.
 *
 * @param {object} opts
 * @param {(frame:object)=>void} opts.send put one rendezvous frame on the wire.
 *   Given the frame body only — `{type:'relay', class, payload, to?}` — so the
 *   caller keeps ownership of the protocol envelope.
 * @param {string} [opts.to] the peer a HOST is addressing. Omitted on the
 *   joiner side, where the service already knows the only possible destination.
 * @param {()=>number} [opts.bufferedAmount] the socket's own send backlog in
 *   bytes. Defaults to 0, which disables shedding — honest for a transport that
 *   cannot report one, rather than a fabricated number.
 * @param {(info:{dropped:number, reason:string})=>void} [opts.onDegraded]
 * @param {(msg:string)=>void} [opts.onLog]
 * @param {object} [opts.limits] from {@link relayLimitsFromFrame}
 */
export function createRelayChannelPair({
  send,
  to = null,
  bufferedAmount = () => 0,
  onDegraded = () => {},
  onLog = () => {},
  limits = RELAY_LIMIT_DEFAULTS,
} = {}) {
  let bounds = limits;
  /** Snapshot frames shed over this pair's whole life, for diagnostics. */
  let dropped = 0;

  function makeChannel(label, cls) {
    const channel = {
      label,
      readyState: 'connecting',
      onopen: null,
      onmessage: null,
      onclose: null,
      onerror: null,
      send(payload) {
        if (channel.readyState !== 'open') return;
        const text = typeof payload === 'string' ? payload : String(payload);
        // Measured by the SAME function the service measures with, so a
        // courtesy refusal here can never disagree with the ceiling there.
        if (relayPayloadBytes(text) > bounds.maxFrameBytes) {
          // Refused locally rather than by being cut off at the service's own
          // ceiling — the same courtesy the DataChannel's max-message-size gets
          // in gui/rendezvous-transport.js, and for the same reason: one lost
          // frame beats a dead link.
          onLog(`[relay] frame too large for the relay (${text.length} bytes) — dropping`);
          return;
        }
        if (cls === RELAY_SNAPSHOT_LABEL && bufferedAmount() > bounds.maxSendBufferBytes) {
          // The lossy class staying lossy. Queuing this behind an existing
          // backlog is exactly the head-of-line blocking the snapshot class
          // exists to avoid, and the next tick supersedes it anyway.
          dropped += 1;
          onDegraded({ dropped, reason: 'send-buffer' });
          return;
        }
        send({ type: 'relay', class: cls, payload: text, ...(to ? { to } : {}) });
      },
      close() {
        if (channel.readyState === 'closed') return;
        channel.readyState = 'closed';
        if (channel.onclose) channel.onclose();
      },
    };
    return channel;
  }

  const reliable = makeChannel(RELAY_RELIABLE_LABEL, RELAY_RELIABLE_LABEL);
  const snapshot = makeChannel(RELAY_SNAPSHOT_LABEL, RELAY_SNAPSHOT_LABEL);

  return {
    reliable,
    snapshot,

    /** Adopt the limits the service advertised. */
    applyLimits(next) {
      bounds = next || bounds;
    },

    /** How many snapshot frames this pair has shed. */
    get droppedSnapshots() {
      return dropped;
    },

    /**
     * Bring both channels up. Called once the service has confirmed the
     * attachment, so `open` means the same thing it means on a DataChannel:
     * the far end can be reached.
     */
    open() {
      for (const channel of [reliable, snapshot]) {
        if (channel.readyState !== 'connecting') continue;
        channel.readyState = 'open';
        if (channel.onopen) channel.onopen();
      }
    },

    /** Hand one inbound `relay` frame to the channel its class names. */
    deliver(frame) {
      const target = frame && frame.class === RELAY_SNAPSHOT_LABEL ? snapshot : reliable;
      if (target.readyState !== 'open' || !target.onmessage) return;
      target.onmessage({ data: frame.payload });
    },

    /** Close both channels, exactly as a peer connection going away would. */
    close() {
      reliable.close();
      snapshot.close();
    },
  };
}

/**
 * A stand-in for the `RTCPeerConnection` a relayed link does not have.
 *
 * gui/rendezvous-transport.js's `connectionAdapter` holds a peer connection so
 * it can close it alongside the channels, and server.html reads
 * `conn.peerConnection` for its own diagnostics. Handing it `null` would make
 * every one of those sites grow a guard; handing it this makes a relayed
 * connection an ordinary one that happens to have no ICE.
 *
 * `iceConnectionState` says `'relayed'` rather than borrowing one of WebRTC's
 * five: a diagnostics readout that printed "connected" here would be claiming
 * an ICE result that was never negotiated.
 */
export function relayPeerStub(onClose = () => {}) {
  return {
    iceConnectionState: 'relayed',
    /** No candidates were ever gathered — there was no ICE. */
    getStats: () => Promise.resolve(new Map()),
    close: onClose,
  };
}
