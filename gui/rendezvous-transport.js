/**
 * gui/rendezvous-transport.js — the Phoenix join path (issue #1111).
 *
 * A parallel, additive route to the PeerJS one: a secure WebSocket to the
 * rendezvous service carries typed join-code lookup and WebRTC signalling, and
 * the game traffic then runs over a direct reliable DataChannel. PeerJS is
 * untouched — replacing it is #1112.
 *
 * Two halves, both here because they are two ends of one frame vocabulary:
 *
 *   createRendezvousHost   server.html — registers, is issued a five-letter
 *                          code, answers offers, and hands each opened channel
 *                          to the page as a PeerJS-shaped connection so the
 *                          existing Identify gate and token maps are reused
 *                          verbatim.
 *   createRendezvousJoiner client.html — resolves a typed code, offers, opens
 *                          the channel, completes the compatibility handshake
 *                          and then speaks ordinary ClientMessage JSON.
 *
 * The compatibility handshake (`JoinHandshake` / `JoinAccepted` /
 * `JoinRefused`) is deliberately NOT a ClientMessage/ServerMessage variant —
 * pasm/spec/design/p2p-design-deltas.yaml forbids layering transport concerns
 * onto the crew protocol. It never reaches WASM as a message; the host answers
 * it from `checkStamp`, which server.html wires to Rust's own
 * `delivery::check_join_stamp`. The rendezvous service's version advice is
 * discovery help only; this handshake is the authority.
 *
 * WebSocket and RTCPeerConnection are taken from an injectable factory pair so
 * the smoke suite can drive the whole path with no real WebRTC (CI has none),
 * and so a native host embedding a webview can supply an in-process adapter
 * later without a second transport module.
 */

import './strings-boot.js';
import { localiseTree } from './strings.js';
import {
  NAMESPACE_CLIENT,
  parseJoinCode,
  reasonStringId,
} from './join-code.js';

/** Frame-vocabulary revision; must match worker-rendezvous/src/registry.js. */
export const RENDEZVOUS_PROTOCOL = 1;

/**
 * The dev rendezvous service. One hardcoded literal, exactly like the TURN
 * worker's at gui/connection-manager.js — .github/workflows/deploy-demo.yml
 * sweeps dist/ for it and patches in the demo worker's URL, so do not build
 * this string from parts.
 */
export const DEV_RENDEZVOUS_URL = 'https://phoenix-rendezvous.project-phoenix.workers.dev';

/** Label of the reliable ordered channel. #1112 adds the lossy 'snapshot' one. */
export const RELIABLE_CHANNEL = 'reliable';

// ── Pure helpers ────────────────────────────────────────────────────────────

/**
 * Which rendezvous service (if any) this page should use.
 *
 * #1111 is additive: the PeerJS route stays the default and the Phoenix route
 * is opted into per page load, so a build whose service is not deployed cannot
 * be broken by it. `?rendezvous` (or `=1`/`=default`/`=on`) uses the built-in
 * URL; `?rendezvous=<url>` points at another one; absent means "off".
 *
 * @returns {string|null} base URL, or null when the Phoenix route is off
 */
export function rendezvousBaseFromLocation(search, defaultBase = DEV_RENDEZVOUS_URL) {
  const params = new URLSearchParams(search || '');
  if (!params.has('rendezvous')) return null;
  const value = (params.get('rendezvous') || '').trim();
  if (!value || value === '1' || value === 'on' || value === 'default') return defaultBase;
  if (value === '0' || value === 'off') return null;
  return value;
}

/**
 * Which join route this client page load is on. Pure, so the branch is decided
 * in one tested place rather than in three `if`s in client.html.
 *
 *   `peerjs`      the fragment is a 32-hex peer id — today's route, untouched
 *   `rendezvous`  the fragment is a structured join code (a QR scan, or a
 *                 pasted full code): join straight away
 *   `entry`       nothing in the fragment: ask for five letters
 *   `none`        nothing in the fragment and no service to ask — the old
 *                 "no host id in the URL" dead end, kept for `?rendezvous=off`
 *
 * A structured code in the fragment IS an opt-in to the Phoenix route, so it
 * does not additionally need `?rendezvous`; that parameter only redirects the
 * page at another service, or turns the route off entirely.
 */
export function joinRouteFromLocation(search, hash, defaultBase = DEV_RENDEZVOUS_URL) {
  const params = new URLSearchParams(search || '');
  const raw = params.has('rendezvous') ? (params.get('rendezvous') || '').trim() : null;
  const off = raw === '0' || raw === 'off';
  const base = off
    ? null
    : raw && raw !== '1' && raw !== 'on' && raw !== 'default'
      ? raw
      : defaultBase;

  const fragment = String(hash || '').replace(/^#/, '');
  if (fragment.includes('_')) {
    return base ? { route: 'rendezvous', base, code: fragment } : { route: 'none' };
  }
  if (fragment) return { route: 'peerjs', hostPeerId: fragment };
  return base ? { route: 'entry', base } : { route: 'none' };
}

/** Socket URL for an endpoint on a service base, upgrading the scheme. */
export function socketUrl(base, path) {
  const url = new URL(path.replace(/^\//, ''), base.endsWith('/') ? base : `${base}/`);
  url.protocol = url.protocol === 'http:' ? 'ws:' : url.protocol === 'https:' ? 'wss:' : url.protocol;
  return url.toString();
}

/**
 * The link a QR encodes and a guest reads aloud: the client page with the full
 * structured code in the fragment. There is no QR *scanner* in the product —
 * the phone's own camera opens this URL — so "QR entry" and "pasted full code"
 * are the same string arriving by two routes.
 */
export function joinUrlForCode(pageHref, fullCode, base = DEV_RENDEZVOUS_URL) {
  const dir = String(pageHref).replace(/[?#].*$/, '').replace(/[^/]*$/, '');
  // Only a non-default service needs saying: the built-in one is what a bare
  // structured code already implies, and a shorter URL is a shorter QR.
  const search = base && base !== DEV_RENDEZVOUS_URL
    ? `?rendezvous=${encodeURIComponent(base)}`
    : '';
  return `${dir}client/index.html${search}#${fullCode}`;
}

/** Default factories; overridable for tests and for a native in-process host. */
export function defaultFactories() {
  const overrides = (typeof window !== 'undefined' && window.PhoenixTransportFactories) || {};
  return {
    socket: overrides.socket || ((url) => new WebSocket(url)),
    peer: overrides.peer || ((config) => new RTCPeerConnection(config)),
  };
}

// ── Shared plumbing ─────────────────────────────────────────────────────────

function frame(type, rest) {
  return JSON.stringify({ v: RENDEZVOUS_PROTOCOL, type, ...rest });
}

/**
 * Wrap a raw RTCDataChannel in the shape server.html's existing per-connection
 * handler expects from a PeerJS DataConnection: `.peer`, `.send`, `.close`,
 * `.on('data'|'close')` and an `open` flag. gui/host-peer-routing.js already
 * accepts either `conn.open` or `conn.readyState === 'open'`, so the outbound
 * router needs no change at all.
 */
function connectionAdapter(peerId, channel, pc) {
  const listeners = { data: [], close: [] };
  const adapter = {
    peer: peerId,
    peerConnection: pc,
    get open() { return channel.readyState === 'open'; },
    get readyState() { return channel.readyState; },
    send(payload) { if (channel.readyState === 'open') channel.send(payload); },
    close() { try { channel.close(); } catch { /* already gone */ } },
    on(event, cb) { (listeners[event] || (listeners[event] = [])).push(cb); },
    emit(event, arg) { for (const cb of listeners[event] || []) cb(arg); },
  };
  channel.onclose = () => adapter.emit('close');
  return adapter;
}

/** Decode one DataChannel payload to an object, or null. */
function decodeFrame(raw) {
  const text = typeof raw === 'string' ? raw : new TextDecoder().decode(raw);
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

// ── Host half ───────────────────────────────────────────────────────────────

/**
 * Register this host with the rendezvous service and accept crew joins.
 *
 * @param {object} opts
 * @param {string} opts.base            rendezvous service base URL
 * @param {string} [opts.namespace]     which code namespace to be issued in
 * @param {string|null} [opts.stamp]    this host's delivery stamp, published as
 *                                      advisory discovery metadata
 * @param {object[]} [opts.iceServers]
 * @param {(stamp:string|null)=>{ok:boolean,code?:string,detail?:string}} [opts.checkStamp]
 *   the authoritative compatibility verdict. Defaults to "admit" so a page that
 *   cannot reach the WASM export degrades to today's behaviour rather than
 *   refusing everyone.
 * @param {(code:object)=>void} [opts.onCode]
 * @param {(conn:object)=>void} [opts.onConnection]
 * @param {(reason:string, detail?:string)=>void} [opts.onError]
 * @param {(msg:string)=>void} [opts.onLog]
 */
export function createRendezvousHost(opts) {
  const {
    base,
    namespace = NAMESPACE_CLIENT,
    stamp = null,
    iceServers = [],
    checkStamp = () => ({ ok: true }),
    onCode = () => {},
    onConnection = () => {},
    onError = () => {},
    onLog = () => {},
    factories = defaultFactories(),
  } = opts;

  const peers = new Map(); // rendezvous peer id → { pc, adapter, pending[] }
  let socket = null;
  let code = null;
  let closed = false;

  function signal(to, payload) {
    if (socket && socket.readyState === 1) socket.send(frame('signal', { to, payload }));
  }

  function peerState(id) {
    let entry = peers.get(id);
    if (entry) return entry;
    const pc = factories.peer({ iceServers });
    entry = { pc, adapter: null, pendingCandidates: [] };
    peers.set(id, entry);
    pc.onicecandidate = (e) => {
      if (e && e.candidate) signal(id, { candidate: e.candidate });
    };
    pc.ondatachannel = (e) => {
      const channel = e.channel;
      if (channel.label !== RELIABLE_CHANNEL) return;
      const adapter = connectionAdapter(id, channel, pc);
      entry.adapter = adapter;
      channel.onmessage = (ev) => {
        const msg = decodeFrame(ev.data);
        // The compatibility handshake is transport-plane and never reaches the
        // page's Identify gate, let alone WASM.
        if (msg && msg.type === 'JoinHandshake') {
          const verdict = checkStamp((msg.data && msg.data.stamp) || null);
          if (verdict.ok) {
            adapter.send(JSON.stringify({ type: 'JoinAccepted', data: {} }));
          } else {
            onLog(`[rendezvous] refusing ${id}: ${verdict.code} ${verdict.detail || ''}`);
            adapter.send(JSON.stringify({
              type: 'JoinRefused',
              data: { code: verdict.code, detail: verdict.detail || '' },
            }));
            // Let the refusal drain before tearing the channel down: a client
            // that is dropped without being told why has learned nothing, and
            // "cannot connect" is the least actionable message in the game.
            setTimeout(() => adapter.close(), 250);
          }
          return;
        }
        adapter.emit('data', ev.data);
      };
      const announce = () => onConnection(adapter);
      if (channel.readyState === 'open') announce();
      else channel.onopen = announce;
    };
    return entry;
  }

  async function onSignal(from, payload) {
    const entry = peerState(from);
    if (payload && payload.sdp) {
      await entry.pc.setRemoteDescription(payload.sdp);
      const answer = await entry.pc.createAnswer();
      await entry.pc.setLocalDescription(answer);
      signal(from, { sdp: entry.pc.localDescription || answer });
      for (const c of entry.pendingCandidates.splice(0)) await entry.pc.addIceCandidate(c);
    } else if (payload && payload.candidate) {
      if (entry.pc.remoteDescription) await entry.pc.addIceCandidate(payload.candidate);
      else entry.pendingCandidates.push(payload.candidate);
    }
  }

  function handle(msg) {
    switch (msg.type) {
      case 'ready':
        socket.send(frame('host-open', { namespace, stamp }));
        break;
      case 'hosted':
        code = msg.code;
        onLog(`[rendezvous] issued ${code.namespace} code ${code.suffix}`);
        onCode(code);
        break;
      case 'peer-joined':
        peerState(msg.peer);
        break;
      case 'peer-left': {
        const entry = peers.get(msg.peer);
        if (entry) {
          if (entry.adapter) entry.adapter.emit('close');
          try { entry.pc.close(); } catch { /* already closed */ }
          peers.delete(msg.peer);
        }
        break;
      }
      case 'signal':
        onSignal(msg.from, msg.payload).catch((e) => onError('signal', String(e && e.message)));
        break;
      case 'error':
        onError(msg.reason, msg.detail);
        break;
      default:
        break;
    }
  }

  function connect() {
    socket = factories.socket(socketUrl(base, '/v1/host'));
    socket.onopen = () => onLog('[rendezvous] host socket open');
    socket.onmessage = (e) => {
      const msg = decodeFrame(e.data);
      if (msg) handle(msg);
    };
    socket.onerror = () => onError('unreachable');
    socket.onclose = () => { if (!closed) onError('unreachable'); };
  }

  connect();

  return {
    get code() { return code; },
    /** Open or close new-joiner admission without dropping the code. */
    setAdmission(state) {
      if (socket && socket.readyState === 1) socket.send(frame('host-admission', { state }));
    },
    close() {
      closed = true;
      if (socket && socket.readyState === 1) socket.send(frame('host-close', {}));
      for (const entry of peers.values()) {
        try { entry.pc.close(); } catch { /* already closed */ }
      }
      peers.clear();
      if (socket) socket.close();
    },
  };
}

// ── Client half ─────────────────────────────────────────────────────────────

/**
 * Resolve a typed code and join the host behind it.
 *
 * `onStatus` mirrors gui/connection-manager.js's vocabulary
 * ('connecting' | 'ready' | 'disconnected' | 'error') so client.html's existing
 * status handling is reused unchanged, and `onError` reports a machine reason
 * that gui/join-code.js's `reasonStringId` turns into display text.
 */
export function createRendezvousJoiner(opts) {
  const {
    base,
    data,
    code,
    namespace = NAMESPACE_CLIENT,
    stamp = null,
    iceServers = [],
    getIdent = () => ({}),
    onData = () => {},
    onStatus = () => {},
    onError = () => {},
    onLog = () => {},
    factories = defaultFactories(),
  } = opts;

  const parsed = parseJoinCode(code, namespace, data);
  if (!parsed.ok) {
    onError(parsed.reason, reasonStringId(parsed.reason));
    return { failed: true, reason: parsed.reason, close() {} };
  }

  let socket = null;
  let pc = null;
  let channel = null;
  let closed = false;
  const pendingCandidates = [];

  function signal(payload) {
    if (socket && socket.readyState === 1) socket.send(frame('signal', { payload }));
  }

  async function offer() {
    pc = factories.peer({ iceServers });
    pc.onicecandidate = (e) => {
      if (e && e.candidate) signal({ candidate: e.candidate });
    };
    channel = pc.createDataChannel(RELIABLE_CHANNEL, { ordered: true });
    channel.onopen = () => {
      // The host's compatibility handshake goes first; Identify follows only
      // once the host has accepted this build.
      channel.send(JSON.stringify({
        type: 'JoinHandshake',
        data: { stamp, code_version: parsed.version },
      }));
    };
    channel.onclose = () => {
      if (!closed) onStatus('disconnected');
    };
    channel.onmessage = (e) => {
      const msg = decodeFrame(e.data);
      if (!msg) return;
      if (msg.type === 'JoinAccepted') {
        onStatus('ready');
        channel.send(JSON.stringify({ type: 'Identify', data: getIdent() }));
        return;
      }
      if (msg.type === 'JoinRefused') {
        const reason = (msg.data && msg.data.code) || 'version-mismatch';
        onLog(`[rendezvous] host refused this build: ${reason}`);
        onError(reason, (msg.data && msg.data.detail) || '');
        onStatus('error');
        closeAll();
        return;
      }
      // Ordinary ServerMessage traffic. localiseTree resolves string ids to
      // display text once, here, exactly as the PeerJS path does — every
      // console downstream assumes it has already happened.
      onData(localiseTree(msg));
    };
    const description = await pc.createOffer();
    await pc.setLocalDescription(description);
    signal({ sdp: pc.localDescription || description });
  }

  async function onSignal(payload) {
    if (payload && payload.sdp) {
      await pc.setRemoteDescription(payload.sdp);
      for (const c of pendingCandidates.splice(0)) await pc.addIceCandidate(c);
    } else if (payload && payload.candidate) {
      if (pc && pc.remoteDescription) await pc.addIceCandidate(payload.candidate);
      else pendingCandidates.push(payload.candidate);
    }
  }

  function handle(msg) {
    switch (msg.type) {
      case 'ready':
        onStatus('connecting');
        socket.send(frame('join', { code: parsed.full, stamp }));
        break;
      case 'joined':
        onLog(`[rendezvous] resolved ${parsed.suffix}; offering`);
        offer().catch((e) => onError('unreachable', String(e && e.message)));
        break;
      case 'signal':
        onSignal(msg.payload).catch((e) => onError('unreachable', String(e && e.message)));
        break;
      case 'closed':
        onError(msg.reason || 'host-gone');
        onStatus('disconnected');
        break;
      case 'error':
        onError(msg.reason, msg.detail);
        onStatus('error');
        break;
      default:
        break;
    }
  }

  function closeAll() {
    closed = true;
    try { if (channel) channel.close(); } catch { /* already gone */ }
    try { if (pc) pc.close(); } catch { /* already gone */ }
    try { if (socket) socket.close(); } catch { /* already gone */ }
  }

  socket = factories.socket(socketUrl(base, '/v1/join'));
  socket.onopen = () => onLog('[rendezvous] join socket open');
  socket.onmessage = (e) => {
    const msg = decodeFrame(e.data);
    if (msg) handle(msg);
  };
  socket.onerror = () => { onError('unreachable'); onStatus('error'); };

  return {
    get suffix() { return parsed.suffix; },
    get full() { return parsed.full; },
    send(type, payload) {
      if (channel && channel.readyState === 'open') {
        channel.send(JSON.stringify(payload !== undefined ? { type, data: payload } : { type }));
      }
    },
    get connected() { return !!channel && channel.readyState === 'open'; },
    close: closeAll,
  };
}

// Expose for classic-script consumers (server.html / client.html).
if (typeof window !== 'undefined') {
  window.rendezvousTransport = {
    RENDEZVOUS_PROTOCOL,
    DEV_RENDEZVOUS_URL,
    RELIABLE_CHANNEL,
    rendezvousBaseFromLocation,
    joinRouteFromLocation,
    socketUrl,
    joinUrlForCode,
    defaultFactories,
    createRendezvousHost,
    createRendezvousJoiner,
  };
}
