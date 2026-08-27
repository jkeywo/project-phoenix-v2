/**
 * gui/rendezvous-transport.js — the Phoenix crew transport (issues #1111, #1112).
 *
 * THE route, since #1112: a secure WebSocket to the rendezvous service carries
 * typed join-code lookup and WebRTC signalling, and the game traffic then runs
 * over direct DataChannels. PeerJS, the peer-id-in-the-URL-hash mechanic and
 * the cloud broker behind them are gone — there is no second transport and no
 * flag selecting between them.
 *
 * Two halves, both here because they are two ends of one frame vocabulary:
 *
 *   createRendezvousHost   server.html — registers, is issued a five-letter
 *                          code, answers offers, and hands each admitted joiner
 *                          to the page as a connection object carrying both of
 *                          its channels, so the page's Identify gate and token
 *                          maps are one code path.
 *   createRendezvousJoiner client.html — resolves a typed code, offers, opens
 *                          the channels, completes the compatibility handshake,
 *                          speaks ordinary ClientMessage JSON, and reconnects
 *                          itself on its own backoff without the guest ever
 *                          re-entering the code.
 *
 * ## Two channels, one connection (#1112 AC2)
 *
 * The joiner is the offerer and creates BOTH channels before the offer, so the
 * answering host picks them up by label:
 *
 *   'reliable'  ordered, retransmitting — commands and every reliable
 *               ServerMessage, plus the join handshake itself.
 *   'snapshot'  `{ordered:false, maxRetransmits:0}` — the snapshot delivery
 *               class (SimState and friends). Genuinely lossy: a late snapshot
 *               is worthless, and head-of-line blocking on a phone's radio is
 *               worse than a dropped frame.
 *
 * Nothing anywhere assumes the lossy channel exists. The host's outbound router
 * (gui/host-peer-routing.js) falls back to the reliable channel PER TOKEN for
 * any client whose snapshot channel has not finished negotiating or has gone,
 * and the joiner's own `send` does the same on the way up.
 *
 * ## The compatibility handshake
 *
 * `JoinHandshake` / `JoinAccepted` / `JoinRefused` are deliberately NOT
 * ClientMessage/ServerMessage variants — pasm/spec/design/p2p-design-deltas.yaml
 * forbids layering transport concerns onto the crew protocol. They never reach
 * WASM as messages; the host answers from `checkStamp`, which server.html wires
 * to Rust's own `delivery::check_join_stamp`. The rendezvous service's version
 * advice is discovery help; this handshake is the authority, and since #1112 a
 * joiner that presents no stamp at all is refused rather than admitted.
 *
 * ## Injected factories
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
import { RENDEZVOUS_PROTOCOL } from './rendezvous-protocol.js';
import {
  nextBackoffDelay,
  connectTimeoutMs,
  candidateType,
} from './connection-manager.js';

/** Re-exported so a consumer of this module needs only one import. */
export { RENDEZVOUS_PROTOCOL };

/**
 * The dev rendezvous service. One hardcoded literal, in the same shape as the
 * TURN worker's at gui/connection-manager.js — keep it a whole string rather
 * than building it from parts, because a deploy-time URL sweep can only find a
 * literal.
 *
 * NOTE, and this is the honest state of it: no such sweep exists for THIS
 * literal yet, and worker-rendezvous/ has never been deployed.
 * .github/workflows/deploy-demo.yml sweeps `DEV_TURN_URL` only. Both are open
 * items in docs/delivery-checklist.md §3a, and since #1112 they are BLOCKING
 * rather than cosmetic: PeerJS is gone, so a build pointed at a service that is
 * not there has no join path at all.
 */
export const DEV_RENDEZVOUS_URL = 'https://phoenix-rendezvous.project-phoenix.workers.dev';

/** Label of the reliable ordered channel: commands and reliable messages. */
export const RELIABLE_CHANNEL = 'reliable';
/** Label of the lossy unordered channel: the snapshot delivery class. */
export const SNAPSHOT_CHANNEL = 'snapshot';

/**
 * Refusals a retry cannot fix. Everything NOT in here — an unreachable
 * service, a signalling drop, an ICE timeout — is a link failure the joiner
 * retries on its own backoff; everything in here is an answer about the CODE
 * or the BUILD, and re-asking gets the same answer while the guest stares at a
 * spinner. Those go back to the entry field with their own sentence.
 *
 * `host-gone` belongs here on purpose: the record died with its host, and the
 * replacement host was issued a different code, so the honest thing is to say
 * so rather than to retry a name that no longer exists (code rebinding across
 * a host drop is #1115's).
 */
const TERMINAL_REASONS = new Set([
  'empty', 'length', 'charset', 'denied', 'malformed',
  'unknown-project', 'unknown-namespace',
  'unknown', 'wrong-type', 'version-mismatch', 'not-joinable',
  'admission-closed', 'host-gone', 'exhausted',
  'too-many-attempts', 'unsupported-protocol', 'forbidden-role', 'not-joined',
  // The host's own StampMismatch::code() values, relayed through JoinRefused.
  'protocol-mismatch', 'content-id-mismatch', 'content-epoch-mismatch',
  'bundle-content-missing', 'client-stamp-missing',
]);

/** True when a machine reason is worth another attempt. */
export function isRetryableReason(reason) {
  return !TERMINAL_REASONS.has(String(reason || ''));
}

// ── Pure helpers ────────────────────────────────────────────────────────────

/**
 * Which rendezvous service this page should use.
 *
 * There is exactly one route now, so this no longer decides WHETHER to use the
 * service — only WHICH one. `?rendezvous=<url>` points a page at another
 * service (a `wrangler dev` on localhost, a staging deployment); anything else,
 * including no parameter at all, means the built-in one. The #1111 opt-in
 * spellings (`?rendezvous`, `=on`, `=1`, `=off`) are gone with PeerJS: a flag
 * whose only remaining value selects the default is a flag that lies about
 * having a choice.
 *
 * A value that is not an http(s) URL is deliberately ignored rather than
 * honoured — an old `?rendezvous=on` bookmark must open the game, not try to
 * dial a service called "on" and throw building the socket URL.
 *
 * @returns {string} base URL — never null, because there is no "off"
 */
export function rendezvousBaseFromLocation(search, defaultBase = DEV_RENDEZVOUS_URL) {
  const value = (new URLSearchParams(search || '').get('rendezvous') || '').trim();
  if (!value) return defaultBase;
  try {
    const { protocol } = new URL(value);
    return protocol === 'http:' || protocol === 'https:' ? value : defaultBase;
  } catch {
    return defaultBase;
  }
}

/**
 * Which join route this client page load is on. Pure, so the branch is decided
 * in one tested place rather than in three `if`s in client.html.
 *
 *   `rendezvous`  something is in the fragment — a QR scan, a pasted full code,
 *                 or a stale bookmark: join with it straight away
 *   `entry`       nothing in the fragment: ask for five letters
 *
 * A fragment that is NOT a valid code is deliberately still `rendezvous`
 * rather than a third route. gui/join-code.js is the one place that decides
 * what a string is, so a stale `#<32 hex peer id>` bookmark from the PeerJS era
 * lands in front of the entry field with a stated reason instead of hanging on
 * a status line, and this function never needs to know what yesterday's links
 * looked like.
 */
export function joinRouteFromLocation(search, hash, defaultBase = DEV_RENDEZVOUS_URL) {
  const base = rendezvousBaseFromLocation(search, defaultBase);
  const fragment = String(hash || '').replace(/^#/, '').trim();
  return fragment ? { route: 'rendezvous', base, code: fragment } : { route: 'entry', base };
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

/** Decode one DataChannel payload to an object, or null. */
function decodeFrame(raw) {
  const text = typeof raw === 'string' ? raw : new TextDecoder().decode(raw);
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

const isChannelOpen = (c) => !!c && c.readyState === 'open';

/**
 * Wrap a joiner's channels in the shape server.html's per-connection handler
 * expects: `.peer`, `.send`, `.close`, `.on('data'|'close'|'snapshot')` and an
 * `open` flag. gui/host-peer-routing.js already accepts either `conn.open` or
 * `conn.readyState === 'open'`, so the outbound router needs no change at all.
 *
 * `snapshotChannel` is the raw lossy `RTCDataChannel` once it has negotiated,
 * and null before that or after it goes; the `'snapshot'` event fires on every
 * transition so the page can keep its per-token map in step without polling.
 */
function connectionAdapter(peerId, channel, pc) {
  const listeners = { data: [], close: [], snapshot: [] };
  const adapter = {
    peer: peerId,
    peerConnection: pc,
    snapshotChannel: null,
    get open() { return channel.readyState === 'open'; },
    get readyState() { return channel.readyState; },
    send(payload) { if (channel.readyState === 'open') channel.send(payload); },
    close() { try { channel.close(); } catch { /* already gone */ } },
    on(event, cb) { (listeners[event] || (listeners[event] = [])).push(cb); },
    emit(event, arg) { for (const cb of listeners[event] || []) cb(arg); },
    /** Adopt the lossy channel negotiated alongside this one. */
    bindSnapshot(chan) {
      if (adapter.snapshotChannel === chan) return;
      adapter.snapshotChannel = isChannelOpen(chan) ? chan : null;
      chan.onopen = () => { adapter.snapshotChannel = chan; adapter.emit('snapshot'); };
      const drop = () => {
        if (adapter.snapshotChannel === chan) adapter.snapshotChannel = null;
        adapter.emit('snapshot');
      };
      chan.onclose = drop;
      chan.onerror = drop;
      adapter.emit('snapshot');
    },
  };
  channel.onclose = () => adapter.emit('close');
  return adapter;
}

// ── Host half ───────────────────────────────────────────────────────────────

/**
 * Register this host with the rendezvous service and accept crew joins.
 *
 * @param {object} opts
 * @param {string} opts.base            rendezvous service base URL
 * @param {string} [opts.namespace]     which code namespace to be issued in
 * @param {object[]} [opts.iceServers]
 * @param {(stamp:string|null)=>{ok:boolean,code?:string,detail?:string}} [opts.checkStamp]
 *   the authoritative compatibility verdict. Defaults to "admit" so a page that
 *   cannot reach the WASM export degrades to admitting rather than refusing
 *   everyone — a host that cannot ask cannot refuse.
 * @param {(code:object)=>void} [opts.onCode] fires again with a NEW code if the
 *   service was lost and this host re-registered.
 * @param {(conn:object)=>void} [opts.onConnection] called ONLY for a joiner the
 *   compatibility handshake admitted — never on channel open.
 * @param {(reason:string, detail?:string)=>void} [opts.onError]
 * @param {(peer:string, state:string)=>void} [opts.onPeerIce] one joiner's ICE
 *   connection state. When ICE never completes no channel ever opens, so this
 *   is the only way the host operator learns a phone is trying and failing.
 * @param {(msg:string)=>void} [opts.onLog]
 * @param {boolean} [opts.reregister] retry registration after service loss
 */
export function createRendezvousHost(opts) {
  const {
    base,
    namespace = NAMESPACE_CLIENT,
    iceServers = [],
    checkStamp = () => ({ ok: true }),
    onCode = () => {},
    onConnection = () => {},
    onError = () => {},
    onPeerIce = () => {},
    onLog = () => {},
    reregister = true,
    factories = defaultFactories(),
  } = opts;

  const peers = new Map(); // rendezvous peer id → per-joiner state
  let socket = null;
  let code = null;
  let closed = false;
  let retryTimer = null;
  let retryAttempt = 0;

  function signal(to, payload) {
    if (socket && socket.readyState === 1) socket.send(frame('signal', { to, payload }));
  }

  function peerState(id) {
    let entry = peers.get(id);
    if (entry) return entry;
    const pc = factories.peer({ iceServers });
    // The gate, per joiner. An open DataChannel is NOT admission: until the
    // compatibility handshake has been answered `ok`, the page has never been
    // handed this connection, so nothing here can reach the Identify gate or
    // wasm_receive_message. Frames arriving before that are DROPPED, not
    // buffered — a build the host is about to refuse has no business queuing
    // simulation traffic — and everything after a refusal is dropped too.
    entry = {
      pc,
      adapter: null,
      snapshot: null,
      admitted: false,
      refused: false,
      pendingCandidates: [],
    };
    peers.set(id, entry);

    pc.onicecandidate = (e) => {
      if (e && e.candidate) signal(id, { candidate: e.candidate });
    };
    pc.oniceconnectionstatechange = () => onPeerIce(id, pc.iceConnectionState);

    /** Hand the lossy channel to the adapter once both exist. */
    function pairSnapshot() {
      if (entry.adapter && entry.snapshot) entry.adapter.bindSnapshot(entry.snapshot);
    }

    /** Ordinary crew traffic, on whichever channel it arrived. */
    function deliver(data) {
      if (entry.refused || !entry.admitted) return;
      const msg = decodeFrame(data);
      // A second handshake from an admitted peer is noise, not a re-vote.
      if (msg && msg.type === 'JoinHandshake') return;
      entry.adapter.emit('data', data);
    }

    pc.ondatachannel = (e) => {
      const channel = e.channel;
      if (channel.label === SNAPSHOT_CHANNEL) {
        entry.snapshot = channel;
        channel.onmessage = (ev) => deliver(ev.data);
        pairSnapshot();
        return;
      }
      if (channel.label !== RELIABLE_CHANNEL) return;
      const adapter = connectionAdapter(id, channel, pc);
      entry.adapter = adapter;
      pairSnapshot();
      channel.onmessage = (ev) => {
        if (entry.refused) return;
        if (!entry.admitted) {
          // The compatibility handshake is transport-plane and never reaches
          // the page's Identify gate, let alone WASM. Nothing else exists on
          // this channel yet.
          const msg = decodeFrame(ev.data);
          if (!msg || msg.type !== 'JoinHandshake') return;
          const verdict = checkStamp((msg.data && msg.data.stamp) || null);
          if (!verdict.ok) {
            entry.refused = true;
            onLog(`[rendezvous] refusing ${id}: ${verdict.code} ${verdict.detail || ''}`);
            adapter.send(JSON.stringify({
              type: 'JoinRefused',
              data: { code: verdict.code, detail: verdict.detail || '' },
            }));
            // Purely a send-flush: let the refusal reach the wire before the
            // channel goes. A client dropped without being told why has
            // learned nothing, and "cannot connect" is the least actionable
            // message in the game. Nothing is admitted during the wait.
            setTimeout(() => adapter.close(), 250);
            return;
          }
          entry.admitted = true;
          // Page first, then the acceptance the joiner answers with Identify,
          // so the handler that reads it is already attached.
          onConnection(adapter);
          adapter.send(JSON.stringify({ type: 'JoinAccepted', data: {} }));
          return;
        }
        deliver(ev.data);
      };
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

  function dropPeers() {
    for (const entry of peers.values()) {
      if (entry.adapter) entry.adapter.emit('close');
      try { entry.pc.close(); } catch { /* already closed */ }
    }
    peers.clear();
  }

  function handle(msg) {
    switch (msg.type) {
      case 'ready':
        socket.send(frame('host-open', { namespace }));
        break;
      case 'hosted':
        code = msg.code;
        retryAttempt = 0;
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
        // Told even for a peer that never opened a channel: it is exactly the
        // struggling-ICE case the diagnostics row is watching, and nothing
        // else would ever clear its line.
        onPeerIce(msg.peer, 'closed');
        break;
      }
      case 'signal':
        onSignal(msg.from, msg.payload).catch((e) => onError('signal', String(e && e.message)));
        break;
      case 'error':
        if (msg.reason === 'unreachable') lostService('unreachable');
        else onError(msg.reason, msg.detail);
        break;
      default:
        break;
    }
  }

  /**
   * The record is gone: the socket died, or the service swept it. There is no
   * second transport to fall back to any more, so a host that simply stopped
   * here would be unjoinable until someone reloaded the viewscreen — which is
   * why this re-registers instead of only reporting.
   *
   * The replacement registration mints a DIFFERENT code, because the old record
   * really is gone and the letters on screen resolve to nothing. Keeping the
   * SAME code across a host drop needs persistence in the service and is
   * #1115's (rotation/rebinding); minting a fresh one and repainting the panel
   * is not that, and it is the difference between a recoverable blip and a
   * dead mission.
   */
  function lostService(reason) {
    if (closed) return;
    // Detached and silenced BEFORE anything else: closing a socket fires its
    // own `close`, and a handler that could still see itself as the current
    // one would re-enter here and report the same loss twice.
    const dying = socket;
    socket = null;
    code = null;
    dropPeers();
    if (dying) {
      dying.onopen = null; dying.onmessage = null; dying.onerror = null; dying.onclose = null;
      try { dying.close(); } catch { /* already gone */ }
    }
    onError(reason || 'unreachable');
    if (!reregister || retryTimer) return;
    const delay = nextBackoffDelay(retryAttempt, 1_000);
    retryAttempt += 1;
    onLog(`[rendezvous] service lost — re-registering in ${delay}ms (attempt ${retryAttempt})`);
    retryTimer = setTimeout(() => { retryTimer = null; if (!closed) connect(); }, delay);
  }

  function connect() {
    socket = factories.socket(socketUrl(base, '/v1/host'));
    const mine = socket;
    socket.onopen = () => onLog('[rendezvous] host socket open');
    socket.onmessage = (e) => {
      if (socket !== mine) return;
      const msg = decodeFrame(e.data);
      if (msg) handle(msg);
    };
    socket.onerror = () => { if (socket === mine) lostService('unreachable'); };
    socket.onclose = () => { if (!closed && socket === mine) lostService('unreachable'); };
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
      if (retryTimer) { clearTimeout(retryTimer); retryTimer = null; }
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
 * Resolve a typed code and join the host behind it, then KEEP that link alive.
 *
 * `onStatus` speaks the page's four states ('connecting' | 'ready' |
 * 'disconnected' | 'error') and `onDiag` the structured connection-diagnostics
 * events client.html's `#conn-diag` readout renders, so the page's existing
 * status handling is reused unchanged. `onError` reports a machine reason that
 * gui/join-code.js's `reasonStringId` turns into display text.
 *
 * ## Reconnect (#1112 AC3)
 *
 * Once the host has accepted this build, a link failure is never terminal: the
 * joiner re-resolves THE SAME code on an exponential backoff, re-offers,
 * re-sends the compatibility handshake and re-sends `Identify` with the same
 * session token — so the host restores the held station and pushes the current
 * projection, and the guest is never asked for five letters a second time.
 * `retryNow()` short-circuits the wait for the page's "retry now" control.
 *
 * Only a refusal a retry cannot fix (see {@link isRetryableReason}) ends the
 * loop and goes back to the entry field with its own sentence.
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
    onDiag = () => {},
    factories = defaultFactories(),
  } = opts;

  const parsed = parseJoinCode(code, namespace, data);
  if (!parsed.ok) {
    onError(parsed.reason, reasonStringId(parsed.reason));
    return {
      failed: true,
      reason: parsed.reason,
      get connected() { return false; },
      send() {},
      retryNow() {},
      close() {},
    };
  }

  let socket = null;
  let pc = null;
  let channel = null;
  let snapshot = null;
  let closed = false;
  /** True once the host has ACCEPTED this build at least once. */
  let established = false;
  let generation = 0;
  /** The attempt already reported as failed, so a late rejection cannot
   *  report it a second time (an in-flight offer whose peer connection was
   *  torn out from under it rejects AFTER the failure that tore it out). */
  let failedGeneration = -1;
  let attemptIndex = 0;
  let retryTimer = null;
  let connectTimer = null;
  let pendingCandidates = [];

  const linked = () => isChannelOpen(channel);

  function clearTimers() {
    if (retryTimer) { clearTimeout(retryTimer); retryTimer = null; }
    if (connectTimer) { clearTimeout(connectTimer); connectTimer = null; }
  }

  /** Drop this attempt's socket/peer/channels without ending the joiner. */
  function teardown() {
    if (connectTimer) { clearTimeout(connectTimer); connectTimer = null; }
    for (const c of [channel, snapshot]) {
      if (!c) continue;
      c.onopen = null; c.onmessage = null; c.onclose = null; c.onerror = null;
      try { c.close(); } catch { /* already gone */ }
    }
    channel = null;
    snapshot = null;
    if (pc) { try { pc.close(); } catch { /* already gone */ } pc = null; }
    if (socket) {
      socket.onopen = null; socket.onmessage = null; socket.onerror = null; socket.onclose = null;
      try { socket.close(); } catch { /* already gone */ }
      socket = null;
    }
    pendingCandidates = [];
  }

  /**
   * This attempt failed. A terminal reason, or a failure before the host ever
   * accepted us, goes back to the page; anything else is the reconnect loop.
   */
  function fail(gen, reason, detail) {
    if (closed || gen !== generation || gen === failedGeneration) return;
    failedGeneration = gen;
    teardown();
    if (established && isRetryableReason(reason)) {
      onLog(`[rendezvous] link lost (${reason}) — retrying`);
      onStatus('disconnected');
      scheduleRetry();
      return;
    }
    onError(reason, detail);
    onStatus(established ? 'disconnected' : 'error');
  }

  function scheduleRetry() {
    if (closed || retryTimer) return;
    const delay = nextBackoffDelay(attemptIndex);
    attemptIndex += 1;
    onLog(`[rendezvous] retrying in ${delay}ms (attempt ${attemptIndex + 1})`);
    retryTimer = setTimeout(() => { retryTimer = null; attempt(); }, delay);
  }

  function signal(payload) {
    if (socket && socket.readyState === 1) socket.send(frame('signal', { payload }));
  }

  async function offer(gen) {
    pc = factories.peer({ iceServers });
    const mine = pc;
    const gathered = new Set();
    pc.onicecandidate = (e) => {
      if (gen !== generation || pc !== mine) return;
      if (!e || !e.candidate) return;
      signal({ candidate: e.candidate });
      const type = candidateType(e.candidate);
      if (type && !gathered.has(type)) {
        gathered.add(type);
        onLog(`[ICE] gathered ${type} candidate`);
        onDiag({ event: 'candidates', types: [...gathered] });
      }
    };
    pc.oniceconnectionstatechange = () => {
      if (gen !== generation || pc !== mine) return;
      onLog(`[ICE] state — ${pc.iceConnectionState}`);
      onDiag({ event: 'ice-state', state: pc.iceConnectionState });
    };

    // Both channels are created here, on the offerer, so the answering host
    // picks them up by label off one negotiation. The lossy one is opened
    // whether or not anything ends up riding it: negotiating it lazily would
    // mean the first minute of every session runs snapshots down the reliable
    // channel for no reason.
    channel = pc.createDataChannel(RELIABLE_CHANNEL, { ordered: true });
    snapshot = pc.createDataChannel(SNAPSHOT_CHANNEL, { ordered: false, maxRetransmits: 0 });
    snapshot.onmessage = (e) => deliver(gen, e.data);
    snapshot.onopen = () => onLog('[rendezvous] snapshot channel open');

    channel.onopen = () => {
      if (gen !== generation) return;
      if (connectTimer) { clearTimeout(connectTimer); connectTimer = null; }
      onDiag({ event: 'open' });
      // The host's compatibility handshake goes first; Identify follows only
      // once the host has accepted this build.
      channel.send(JSON.stringify({
        type: 'JoinHandshake',
        data: { stamp, code_version: parsed.version },
      }));
    };
    channel.onclose = () => {
      if (closed || gen !== generation) return;
      fail(gen, 'unreachable');
    };
    channel.onmessage = (e) => {
      if (gen !== generation) return;
      const msg = decodeFrame(e.data);
      if (!msg) return;
      if (msg.type === 'JoinAccepted') {
        established = true;
        attemptIndex = 0;
        onStatus('ready');
        // Re-sent on EVERY reconnect, not just the first: the host restores
        // seat, rating and projection from the token on every Identify, which
        // is what makes an automatic reconnect resume the same station.
        channel.send(JSON.stringify({ type: 'Identify', data: getIdent() }));
        return;
      }
      if (msg.type === 'JoinRefused') {
        const reason = (msg.data && msg.data.code) || 'version-mismatch';
        onLog(`[rendezvous] host refused this build: ${reason}`);
        // A refusal is the host's answer about this BUILD; established or not,
        // reconnecting would only be told the same thing again.
        established = false;
        fail(gen, reason, (msg.data && msg.data.detail) || '');
        return;
      }
      deliver(gen, e.data);
    };

    // `mine`, not `pc`, from here down: this function is async, and a failure
    // during the awaits tears `pc` out from under it. Reading the field would
    // then throw on null and report the same failure a second time.
    const description = await mine.createOffer();
    await mine.setLocalDescription(description);
    if (gen !== generation) return;
    signal({ sdp: mine.localDescription || description });
  }

  /** Ordinary ServerMessage traffic, on whichever channel it arrived. */
  function deliver(gen, raw) {
    if (gen !== generation) return;
    const msg = decodeFrame(raw);
    if (!msg) return;
    // localiseTree resolves string ids to display text once, here, so no
    // console downstream has to know which of its fields are localisable.
    onData(localiseTree(msg));
  }

  function handle(gen, msg) {
    switch (msg.type) {
      case 'ready':
        if (!established) onStatus('connecting');
        onDiag({ event: 'signaling', state: 'open' });
        // The joiner's own stamp travels in the in-band JoinHandshake, to the
        // host that decides on it — never through the service, which has no
        // say and would only be relaying dead metadata.
        socket.send(frame('join', { code: parsed.full }));
        break;
      case 'joined':
        onLog(`[rendezvous] resolved ${parsed.suffix}; offering`);
        offer(gen).catch((e) => fail(gen, 'unreachable', String(e && e.message)));
        break;
      case 'signal':
        onSignal(gen, msg.payload).catch((e) => fail(gen, 'unreachable', String(e && e.message)));
        break;
      // Both of these are answers, not blips: the record is gone, or the
      // service refused the request. Tear the attempt down as well as
      // reporting it, so a dead attempt can never fire a page callback later.
      case 'closed':
        fail(gen, msg.reason || 'host-gone');
        break;
      case 'error':
        fail(gen, msg.reason, msg.detail);
        break;
      default:
        break;
    }
  }

  async function onSignal(gen, payload) {
    if (gen !== generation || !pc) return;
    if (payload && payload.sdp) {
      await pc.setRemoteDescription(payload.sdp);
      for (const c of pendingCandidates.splice(0)) await pc.addIceCandidate(c);
    } else if (payload && payload.candidate) {
      if (pc.remoteDescription) await pc.addIceCandidate(payload.candidate);
      else pendingCandidates.push(payload.candidate);
    }
  }

  /** One resolve-offer-handshake-Identify pass. Every retry runs this again. */
  function attempt() {
    if (closed) return;
    teardown();
    const gen = ++generation;
    // 'connecting' only while the guest is still waiting to get IN. Once the
    // host has accepted this build the page is in its reconnect loop, and
    // reporting 'connecting' on every attempt would flash the status dot green
    // and hide the retry control several times a second — a link that is down
    // and being retried reads as "reconnecting" until it is actually back.
    if (!established) onStatus('connecting');
    onDiag({ event: 'attempt', attempt: attemptIndex + 1 });
    onDiag({ event: 'signaling', state: 'connecting' });

    const timeoutMs = connectTimeoutMs(attemptIndex);
    connectTimer = setTimeout(() => {
      connectTimer = null;
      if (closed || gen !== generation || linked()) return;
      onLog(`[rendezvous] attempt ${attemptIndex + 1} timed out after ${timeoutMs}ms`);
      onDiag({ event: 'timeout', attempt: attemptIndex + 1 });
      fail(gen, 'unreachable');
    }, timeoutMs);

    socket = factories.socket(socketUrl(base, '/v1/join'));
    const mine = socket;
    socket.onopen = () => onLog('[rendezvous] join socket open');
    socket.onmessage = (e) => {
      if (gen !== generation || socket !== mine) return;
      const msg = decodeFrame(e.data);
      if (msg) handle(gen, msg);
    };
    // An abnormal WS termination fires `error` BEFORE `close` (MDN), and a
    // server-initiated close — DO eviction, worker redeploy, idle timeout, an
    // LB reset — fires `close` with no preceding `error`. Both mean the same
    // thing here, and neither means anything once the direct channel is up:
    // afterwards the signalling socket going away is expected, not a failure.
    const socketGone = () => {
      if (closed || gen !== generation || socket !== mine || linked()) return;
      fail(gen, 'unreachable');
    };
    socket.onerror = socketGone;
    socket.onclose = socketGone;
  }

  attempt();

  return {
    get suffix() { return parsed.suffix; },
    get full() { return parsed.full; },
    get connected() { return linked(); },
    /**
     * Send one ClientMessage. `deliveryClass === 'snapshot'` prefers the lossy
     * channel and falls back to the reliable one when it is not up — the same
     * per-link rule gui/host-peer-routing.js applies per token on the way down.
     */
    send(type, payload, deliveryClass) {
      const json = JSON.stringify(payload !== undefined ? { type, data: payload } : { type });
      if (deliveryClass === 'snapshot' && isChannelOpen(snapshot)) {
        try {
          snapshot.send(json);
          return;
        } catch { /* fall through to the reliable channel */ }
      }
      if (isChannelOpen(channel)) channel.send(json);
    },
    /** The page's "retry now" control: skip the backoff wait and go again. */
    retryNow() {
      if (closed || linked()) return;
      clearTimers();
      attempt();
    },
    close() {
      closed = true;
      clearTimers();
      teardown();
    },
  };
}

// Expose for classic-script consumers (server.html / client.html).
if (typeof window !== 'undefined') {
  window.rendezvousTransport = {
    RENDEZVOUS_PROTOCOL,
    DEV_RENDEZVOUS_URL,
    RELIABLE_CHANNEL,
    SNAPSHOT_CHANNEL,
    isRetryableReason,
    rendezvousBaseFromLocation,
    joinRouteFromLocation,
    socketUrl,
    joinUrlForCode,
    defaultFactories,
    createRendezvousHost,
    createRendezvousJoiner,
  };
}
