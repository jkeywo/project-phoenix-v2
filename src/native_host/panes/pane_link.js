// Pane link — the LAST script in a pane document (issue #1122).
//
// Native-side glue, not client code. It hands the client page an in-process
// stand-in for the Phoenix crew transport, so the page runs its ORDINARY join
// and is a joiner like any other in its own eyes.
//
// # The seam, and why it is this one
//
// gui/rendezvous-transport.js takes its `WebSocket` and its `RTCPeerConnection`
// from `defaultFactories()`, which reads `window.PhoenixTransportFactories` on
// every call — a documented override point whose stated purpose is "a native
// in-process host". Overriding it is the whole of the pane's transport:
//
//   * client.html's `startPhoenixJoin` runs unchanged, so the page owns its own
//     `activeLink`/`window.phoenixLink` façade, its status line, its
//     `#conn-diag` readout and its retry control — none of which a pane-specific
//     link object could reach, because `currentLink()` reads a closure;
//   * `Identify` is sent by the page's own joiner, on the page's own
//     `getIdent()`, once the handshake below has accepted it;
//   * inbound crosses the joiner's own `deliver`, so `localiseTree` is applied
//     exactly where a phone applies it and `handleMessage` is handed exactly
//     what a phone's is.
//
// Nothing here reimplements a page contract, which is what the previous
// arrangement did: it published `window.connectionManager`, a global issue
// #1112 retired along with PeerJS, and the page therefore never called it.
//
// # What the stand-ins stand in for
//
// There is no service and no peer. The socket answers the two frames the joiner
// waits on (`ready`, then `joined` for its `join`) and relays nothing, because
// there is no second party to relay to. The peer connection's reliable channel
// opens immediately, answers the compatibility handshake itself — a pane loads
// the very bundle this process is serving, so there is no version to disagree
// about — and from then on is a pipe onto `window.phoenixPaneOut` in one
// direction and `window.__phoenixPaneApply` in the other.
//
// It is a module, and last in <body>, so that the channel labels are IMPORTED
// from the transport rather than copied: a pane that guessed 'reliable' would
// keep working right up until the transport renamed it.
import {
  RELIABLE_CHANNEL,
  SNAPSHOT_CHANNEL,
  RENDEZVOUS_PROTOCOL,
} from './gui/rendezvous-transport.js';

const pane = window.__phoenixPane || {
  token: '',
  name: '',
  code: '',
  deliver: null,
  drainInbox() {},
};

/** JSON, or null. The page's own frames are the only things that arrive here. */
function decode(raw) {
  try {
    return JSON.parse(typeof raw === 'string' ? raw : new TextDecoder().decode(raw));
  } catch {
    return null;
  }
}

/**
 * The one message the pane rewrites on the way out.
 *
 * The page computes its token from the `session-token` key pane_boot.js seeded,
 * so ordinarily this changes nothing. It is here for the case where it does:
 * storage can be refused, and a pane presenting some other token would be
 * refused at `PaneBus::submit` and would simply never join, with a clean log on
 * both sides. A pane may only ever identify as itself.
 */
function outboundJson(msg, raw) {
  if (!msg || msg.type !== 'Identify') return raw;
  return JSON.stringify({ type: 'Identify', data: { token: pane.token, name: pane.name } });
}

/** An in-process stand-in for one `RTCDataChannel`. */
function paneChannel(label) {
  const chan = {
    label,
    readyState: 'connecting',
    onopen: null,
    onmessage: null,
    onclose: null,
    onerror: null,
    send(payload) {
      const msg = decode(payload);
      // The compatibility handshake is transport-plane and never reaches the
      // host: a pane and its host are one process running one bundle.
      if (msg && msg.type === 'JoinHandshake') {
        accept(chan);
        return;
      }
      window.phoenixPaneOut.send(outboundJson(msg, payload));
    },
    /** What `accept` wired the host's pushes to, so this can unwire exactly it. */
    delivery: null,
    close() {
      if (chan.readyState === 'closed') return;
      chan.readyState = 'closed';
      // Identity-guarded, as the real adapter's snapshot handling is: a channel
      // being torn down must not silence a later one that has taken over.
      if (chan.delivery && pane.deliver === chan.delivery) pane.deliver = null;
      if (typeof chan.onclose === 'function') chan.onclose({});
    },
    /** Host -> page, on whichever channel the page is listening to. */
    receive(payload) {
      if (chan.readyState !== 'open' || typeof chan.onmessage !== 'function') return;
      chan.onmessage({ data: payload });
    },
    open() {
      if (chan.readyState !== 'connecting') return;
      chan.readyState = 'open';
      if (typeof chan.onopen === 'function') chan.onopen({});
    },
  };
  return chan;
}

/**
 * Admit this page, then start delivering to it.
 *
 * On a later turn, because that is when a real host's answer would arrive and
 * because the joiner is still inside its own `onopen` when the handshake is
 * sent. The joiner answers `JoinAccepted` with `Identify`, so by the time
 * `deliver` is wired the host has been told who this is — and the backlog that
 * queued while the document was loading goes out immediately after.
 */
function accept(chan) {
  // The handshake is a reliable-channel frame by definition, and `deliver` is
  // wired to whichever channel carried it — so a lossy channel that somehow
  // asked to be admitted would silently become the pane's inbound path.
  if (chan.label !== RELIABLE_CHANNEL) return;
  setTimeout(() => {
    chan.receive(JSON.stringify({ type: 'JoinAccepted', data: {} }));
    chan.delivery = (json) => chan.receive(json);
    pane.deliver = chan.delivery;
    pane.drainInbox();
  }, 0);
}

/** An in-process stand-in for one `RTCPeerConnection`. */
function panePeer() {
  const channels = [];
  const pc = {
    iceConnectionState: 'connected',
    localDescription: null,
    remoteDescription: null,
    onicecandidate: null,
    oniceconnectionstatechange: null,
    ondatachannel: null,
    createDataChannel(label) {
      if (label !== RELIABLE_CHANNEL && label !== SNAPSHOT_CHANNEL) {
        console.warn(`[pane] the page opened a channel this transport does not know: ${label}`);
      }
      const chan = paneChannel(label);
      channels.push(chan);
      // Opened on a later turn: the joiner attaches `onopen`, `onclose` and
      // `onmessage` in the statements after this call returns.
      setTimeout(() => chan.open(), 0);
      return chan;
    },
    createOffer() {
      return Promise.resolve({ type: 'offer', sdp: 'pane' });
    },
    setLocalDescription(description) {
      pc.localDescription = description || null;
      return Promise.resolve();
    },
    setRemoteDescription(description) {
      pc.remoteDescription = description || null;
      return Promise.resolve();
    },
    addIceCandidate() {
      return Promise.resolve();
    },
    close() {
      // Each channel unwires the delivery it installed, and only that one.
      for (const chan of channels.splice(0)) chan.close();
    },
  };
  return pc;
}

/** An in-process stand-in for the rendezvous signalling socket. */
function paneSocket() {
  const sock = {
    readyState: 1,
    onopen: null,
    onmessage: null,
    onerror: null,
    onclose: null,
    send(raw) {
      const frame = decode(raw);
      // A `join` is the only frame with an answer; `signal` has nowhere to go.
      if (frame && frame.type === 'join') sock.emit({ type: 'joined' });
    },
    close() {
      sock.readyState = 3;
    },
    emit(frame) {
      setTimeout(() => {
        if (sock.readyState !== 1 || typeof sock.onmessage !== 'function') return;
        sock.onmessage({ data: JSON.stringify({ v: RENDEZVOUS_PROTOCOL, ...frame }) });
      }, 0);
    },
  };
  setTimeout(() => {
    if (typeof sock.onopen === 'function') sock.onopen({});
    sock.emit({ type: 'ready' });
  }, 0);
  return sock;
}

// The override itself. Read by `defaultFactories()` on every joiner the page
// creates, so it has only to be here before `startPhoenixJoin` runs — which is
// after DOMContentLoaded, well after this module has evaluated.
window.PhoenixTransportFactories = {
  socket: () => paneSocket(),
  peer: () => panePeer(),
};

// The page fetches ICE servers before connecting and probes a TURN relay for
// its diagnostics readout. A pane has no WebRTC connection to configure, and
// both would otherwise reach for the network — on a bridge machine that may
// have none — and stall the boot behind a timeout.
window.fetchIceServers = async () => ({ servers: [], relayAvailable: false, relaySource: null });
window.probeTurnRelay = async () => 'unreachable';
