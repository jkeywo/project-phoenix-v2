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
 *   createRendezvousHost   server.html — registers, is issued a typed
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
 * ## Two planes, and they are independent
 *
 * The SIGNALLING plane is the rendezvous socket and its frames: registration,
 * code lookup, presence, the SDP/ICE relay, `closed`, `error`. The MEDIA plane
 * is an established `RTCPeerConnection` and its two DataChannels. Getting on
 * the wire needs both; STAYING on it needs only the second. Once the channels
 * are up they are a direct browser-to-browser link, and the phone cannot even
 * observe what the service does afterwards.
 *
 * So a signalling event never touches an established link, on either half:
 *
 *   host    `lostService()` discards the socket, the code and the peers still
 *           mid-signalling, then re-registers so NEW joiners have a way in.
 *           Admitted adapters and their peer connections are untouched — a
 *           Durable Object eviction is not a reason to end a mission. A
 *           `peer-left` for one already-admitted, still-open joiner is the
 *           same story at per-peer scale: the registry sends it because THAT
 *           joiner's own rendezvous socket died, not its DataChannel, so its
 *           link stays up too — only the reliable channel's own close (or a
 *           page-initiated `close()`) may end that session.
 *   joiner  a `closed`/`error` frame, or the signalling socket dying, is
 *           logged and ignored while `linked()`. Only the DataChannel's own
 *           close drives the reconnect loop, and only `close()` (or the host
 *           severing the connection) ends a session.
 *
 * Both halves got this wrong at first, and both wrongnesses read as the same
 * bug from the bridge: a service blip disconnecting everybody mid-mission.
 *
 * ## The third rung (#1113)
 *
 * Some networks build no direct link at any price. When the WebRTC ladder is
 * spent, the joiner asks the service to carry the game's own frames over the
 * rendezvous socket instead (`relay-open`, gui/rendezvous-relay.js) — and for
 * a peer on THAT path the two planes above collapse back into one: the
 * signalling socket IS the link, so `closed`, `error` and the socket's own
 * death are link failures again rather than news. `socketIsTheLink()` on the
 * joiner and `entry.relay` on the host are the two places that exception is
 * spelled out; everything else about a relayed peer — the compatibility
 * handshake, the Identify gate, the adapter, the delivery classes — is the
 * ordinary code path, deliberately, because a fallback with its own admission
 * gate would be a hole the direct path does not have.
 *
 * gui/transport-levers.js can pin any one rung for a test or a field check.
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
  getJoinCodeData,
} from './join-code.js';
import { RENDEZVOUS_PROTOCOL } from './rendezvous-protocol.js';
import {
  nextBackoffDelay,
  connectTimeoutMs,
  candidateType,
  readSelectedPair,
} from './connection-manager.js';
import {
  createRelayChannelPair,
  relayLimitsFromFrame,
  relayPeerStub,
} from './rendezvous-relay.js';
import { defaultTransportLevers } from './transport-levers.js';
import {
  DEV_RENDEZVOUS_URL,
  KNOWN_WEB_ORIGINS,
  joinUrlForCode,
  rendezvousBaseForOrigin,
} from './join-url.js';

/** Re-exported so a consumer of this module needs only one import. */
export { RENDEZVOUS_PROTOCOL };

/**
 * The service's own URL and the join link a code points at both live in
 * `gui/join-url.js` now (issue #1329), and are re-exported here unchanged so
 * that every importer of this module — and `window.rendezvousTransport` — sees
 * them exactly where they were.
 *
 * They moved because the native host's lobby surface has to build a join URL (it
 * draws the QR) while speaking no WebRTC whatsoever: its host does the transport
 * in Rust. Loading this whole module there, to concatenate a string, would have
 * been the wrong dependency in the wrong direction.
 */
export { DEV_RENDEZVOUS_URL, KNOWN_WEB_ORIGINS, joinUrlForCode, rendezvousBaseForOrigin };

/**
 * The service THIS page should dial when nothing overrides it (issue #1353).
 *
 * A function rather than a constant because it is a fact about the page, and
 * the two exported entry points below take it as a default-parameter
 * expression — evaluated per call, so a test may drive them with an explicit
 * base and a page never has to remember to pass one.
 *
 * Off a browser entirely (the Node test environment, a worker) there is no
 * origin to have been served by, so the built-in service stands.
 */
function pageRendezvousBase() {
  const origin = typeof location !== 'undefined' && location ? location.origin : '';
  return rendezvousBaseForOrigin(origin);
}

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
 * `host-gone` belongs here on purpose: it is only ever sent once a record is
 * genuinely, finally gone — the reclaim grace window ran out unclaimed, the
 * host closed deliberately, or the operator rotated the code (issue #1115;
 * see worker-rendezvous/src/registry.js's `dropRecord`) — so the honest thing
 * is to say so rather than to retry a name that no longer exists. A record
 * merely grace-held after a transient socket loss answers a `join` with the
 * RETRYABLE `unreachable` instead (not in this set), which is what lets a
 * joiner's own reconnect loop ride out an ordinary blip without ever seeing
 * this terminal answer.
 */
const TERMINAL_REASONS = new Set([
  'empty', 'length', 'charset', 'denied', 'malformed',
  'unknown-project', 'unknown-namespace',
  'unknown', 'wrong-type', 'version-mismatch', 'not-joinable',
  'admission-closed', 'host-gone', 'exhausted',
  'too-many-attempts', 'unsupported-protocol', 'forbidden-role', 'not-joined',
  // The fleet owner's own answers (gui/host-mesh.js, issue #1114). Both are
  // statements about the FLEET rather than about the link, and re-offering
  // gets the same one: a full fleet does not empty because somebody retried,
  // and a launched mission does not un-launch.
  //
  // Neither reaches `fail()` today — they arrive as host-mesh `refused` frames
  // on an already-open channel, and it is server.html's `leaveFleet` that stops
  // the loop. They are listed anyway because this set is the project's one
  // answer to "is this reason worth another attempt", and a future path that
  // does route a fleet refusal through the joiner must not learn a second one.
  'fleet-full', 'recovery-only',
  // The host's own StampMismatch::code() values, relayed through JoinRefused,
  // plus the native host's refusal of an invalid or reserved token — which
  // this page would present again, identically, on every retry.
  'protocol-mismatch', 'content-id-mismatch', 'content-epoch-mismatch',
  'bundle-content-missing', 'client-stamp-missing', 'reserved-token', 'invalid-token',
]);

/** True when a machine reason is worth another attempt. */
export function isRetryableReason(reason) {
  return !TERMINAL_REASONS.has(String(reason || ''));
}

/**
 * How many attempts a guest who has NEVER been admitted gets before the join
 * entry field comes back carrying the reason.
 *
 * Four, and the number is what makes the escalating connect timeout reachable
 * on a FIRST join. `connectTimeoutMs` runs 8 s, 16 s, then 30 s thereafter,
 * and it escalates for exactly the case a first join meets: TURN-over-TCP
 * allocation on a cellular network, which regularly needs longer than eight
 * seconds. One attempt and out — which is what "retry only once established"
 * meant in practice — put that guest in front of the entry field reading
 * "cannot reach the join service" for a link that would have come up on the
 * second try, and the 16 s and 30 s rungs of the ladder were unreachable by
 * anyone who had not already got in once.
 *
 * Bounded rather than endless, though, because a guest who has never been
 * admitted may simply be reading the wrong code off the viewscreen,
 * and a silent backoff tells them nothing. Four attempts is roughly a minute
 * and a half of trying before the field comes back; after acceptance the loop
 * is unbounded, because the code is known good.
 */
export const JOIN_ATTEMPTS_BEFORE_ENTRY = 4;

// ── Pure helpers ────────────────────────────────────────────────────────────

/**
 * Loopback hostnames — the only origins the `?rendezvous` override accepts.
 * `new URL('http://[::1]:8787').hostname` keeps its brackets, so both
 * spellings are listed rather than normalised.
 */
function isLoopbackHost(hostname) {
  const h = String(hostname || '').toLowerCase();
  return h === 'localhost' || h === '127.0.0.1' || h === '[::1]' || h === '::1'
    || h.endsWith('.localhost');
}

/**
 * Which rendezvous service this page should use.
 *
 * There is exactly one route now, so this no longer decides WHETHER to use the
 * service — only WHICH one. `?rendezvous=<url>` is a DEVELOPMENT lever: it is
 * honoured only for a loopback origin (`localhost`, `127.0.0.1`, `[::1]`, or a
 * `*.localhost` name) — a `wrangler dev`, or a second service on the same
 * machine. Every other value, a public staging URL included, falls back to the
 * built-in service. The #1111 opt-in spellings (`?rendezvous`, `=on`, `=1`,
 * `=off`) are gone with PeerJS: a flag whose only remaining value selects the
 * default is a flag that lies about having a choice.
 *
 * The loopback restriction is what the parameter's post-#1112 life needs.
 * Under #1111 this was also the route opt-in, so it only mattered to somebody
 * deliberately turning the new route on; since #1112 it applies on every
 * ordinary load of client.html. A link of the form
 * `client/index.html?rendezvous=https://attacker.example#<code>` handed to a
 * guest would otherwise send their SDP, their ICE candidates and — through a
 * hostile relay standing in as the host — their session token and display name
 * to a third party, with nothing on screen saying where the join went. A
 * loopback origin cannot be handed to somebody else's phone, which is exactly
 * the property wanted from a dev lever.
 *
 * A value that is not an http(s) URL is likewise ignored rather than honoured
 * — an old `?rendezvous=on` bookmark must open the game, not try to dial a
 * service called "on" and throw building the socket URL.
 *
 * The DEFAULT is no longer always the cloud service (issue #1353): a page
 * served by a native host dials that host, which is
 * {@link rendezvousBaseForOrigin}'s rule, read off this page's own origin. The
 * parameter still wins where it is honoured — a `wrangler dev` on loopback is
 * exactly the case a developer overrides FOR — and the rule needs no parameter
 * of its own, so no link can point a guest's join anywhere.
 *
 * @returns {string} base URL — never null, because there is no "off"
 */
export function rendezvousBaseFromLocation(search, defaultBase = pageRendezvousBase()) {
  const value = (new URLSearchParams(search || '').get('rendezvous') || '').trim();
  if (!value) return defaultBase;
  try {
    const { protocol, hostname } = new URL(value);
    if (protocol !== 'http:' && protocol !== 'https:') return defaultBase;
    return isLoopbackHost(hostname) ? value : defaultBase;
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
 *   `entry`       nothing in the fragment: ask for the code
 *
 * A fragment that is NOT a valid code is deliberately still `rendezvous`
 * rather than a third route. gui/join-code.js is the one place that decides
 * what a string is, so a stale `#<32 hex peer id>` bookmark from the PeerJS era
 * lands in front of the entry field with a stated reason instead of hanging on
 * a status line, and this function never needs to know what yesterday's links
 * looked like.
 */
export function joinRouteFromLocation(search, hash, defaultBase = pageRendezvousBase()) {
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

// `joinUrlForCode` moved to gui/join-url.js (issue #1329) and is imported and
// re-exported above.

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
 *
 * `hooks.onSever` runs when the PAGE closes this connection, so the host half
 * can mark the peer refused; `hooks.onLog` carries a failed send.
 */
function connectionAdapter(peerId, channel, pc, hooks = {}) {
  const onSever = hooks.onSever || (() => {});
  const onLog = hooks.onLog || (() => {});
  const listeners = { data: [], close: [], snapshot: [] };
  const adapter = {
    peer: peerId,
    peerConnection: pc,
    snapshotChannel: null,
    get open() { return channel.readyState === 'open'; },
    get readyState() { return channel.readyState; },
    /**
     * One reliable-channel send, and it may not throw.
     *
     * server.html's `routeOutbound` walks the whole target list inside the
     * callback Rust's outbound flush invokes, so an exception here would abort
     * delivery to every remaining crew member — not just to this one. Two
     * things can raise it: a channel that died between the map lookup and the
     * send, and a payload over the SDP-negotiated `max-message-size` (262144
     * bytes between two Chromiums, and the smallest ceiling any browser pair
     * negotiates in practice). This is a raw `RTCDataChannel` with no chunking
     * layer under it — PeerJS used to provide one — so that ceiling is real.
     * Where the shipped payload classes actually stand against it, since a
     * bare "it's fine" is worth nothing: the two that scale with content are
     * `WorldData` (one EntitySnapshot per STATIC entity, and the largest world
     * in assets/worlds authors 17 `[[entity]]` blocks) and the per-tick
     * snapshot (dynamic entities — hundreds of asteroids at worst, a few tens
     * of KB). Mod packs never cross this wire at all: the host reads the ZIP
     * locally and clients fetch content over HTTP. So a throw here is a dead
     * link or a bug rather than an ordinary payload — and if a future world
     * ever does cross the ceiling, ONE client loses that frame, visibly,
     * instead of the whole bridge silently losing the flush.
     */
    send(payload) {
      if (channel.readyState !== 'open') return;
      try {
        channel.send(payload);
      } catch (e) {
        onLog(`[rendezvous] send to ${peerId} failed — dropping this frame: ${e && e.message}`);
      }
    },
    /**
     * Sever this joiner: both channels AND the RTCPeerConnection.
     *
     * server.html uses this as its only eviction mechanism — the reserved-token
     * refusal ("refuse the connection outright rather than dispatch a single
     * message under it") and the duplicate-token dance ("its WebRTC link is
     * severed"). Closing the reliable channel alone left the lossy one open
     * with its own inbound handler still wired to `deliver()`, so an evicted
     * device could keep dispatching into the simulation down the other pipe.
     * `onSever` marks the host-side entry refused as well, so anything already
     * in flight on either channel is dropped rather than delivered.
     */
    close() {
      onSever();
      const lossy = adapter.snapshotChannel;
      adapter.snapshotChannel = null;
      for (const c of [channel, lossy]) {
        if (!c) continue;
        try { c.close(); } catch { /* already gone */ }
      }
      try { pc.close(); } catch { /* already gone */ }
    },
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
 * @param {(peer:string, dropped:number, source:'local'|'service')=>void} [opts.onPeerShedding]
 *   how many snapshot frames this relayed joiner's link has shed, cumulative.
 *   Its OWN callback rather than another `onPeerIce` state: those two facts
 *   are both true at once, and folding the count into the ICE-state map
 *   replaced the "carried by the join service" row with a raw
 *   `relay-shedding-3` token in an operator-facing line. `source` names WHICH
 *   queue shed: `'local'` is this host's own pair, shed against its send
 *   buffer before a frame ever reaches the service; `'service'` is the
 *   service shedding this host's own outbound frames at the (host,peer)
 *   mailbox before they reach the peer. Both are this host's own downlink to
 *   THAT peer, measured at two different hops — a caller that folds them with
 *   `Math.max` instead of keeping them apart and adding is undercounting
 *   exactly the way `gui/connection-diagnostics.js`'s `relayDroppedBy` used to.
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
    onPeerShedding = () => {},
    onLog = () => {},
    reregister = true,
    levers = defaultTransportLevers(),
    factories = defaultFactories(),
  } = opts;

  /**
   * The RTCPeerConnection config, in one place so the `?transport` levers
   * (gui/transport-levers.js) reach every peer connection this host answers
   * with. Applying them on the HOST as well as the joiner matters in both
   * directions: ICE only negotiates a relay pair if BOTH ends offer relay
   * candidates, and it can only avoid one if neither end has a TURN server to
   * allocate from — so a lever applied to one side alone proves nothing.
   */
  const peerConfig = () => ({
    iceServers: levers.useIceServers === false ? [] : iceServers,
    iceTransportPolicy: levers.iceTransportPolicy,
  });

  const peers = new Map(); // rendezvous peer id → per-joiner state
  let socket = null;
  let code = null;
  let closed = false;
  let retryTimer = null;
  let retryAttempt = 0;
  /**
   * This host's reclaim secret (issue #1115) — the suffix + secret pair its
   * own earlier `hosted` frame carried, held across `lostService()` so the
   * NEXT registration can ask for the SAME code back. Deliberately NOT
   * cleared alongside `code` in `lostService()`: `code` is what the page
   * paints (blank while there is genuinely nothing to show), `resumeToken`
   * is what the wire presents on the way back in, and the two have to
   * survive independently for reclaim to work at all.
   */
  let resumeToken = null;
  /**
   * When the CURRENT run of service loss began (issue #1115) — set on the first
   * `lostService` of a loss streak, held across its failed retries, cleared the
   * moment a `hosted` frame re-registers this host. It exists so `resuming`
   * (below) can tell an in-flight reclaim from one that has already lost: past
   * the authored `reclaim_grace_seconds` the registry has dropped the held
   * record for good, so the next `hosted` will carry a FRESH suffix, and the
   * "reconnecting your code" line must stop claiming otherwise.
   */
  let serviceLostAt = null;

  /**
   * The authored grace window in ms, read live from the loaded join-code table
   * (its `[limits] reclaim_grace_seconds`). Defaults to the registry's own
   * fallback when the table is absent or predates the field, so the two ends
   * agree on the same number without this module hardcoding it.
   */
  function graceWindowMs() {
    const data = getJoinCodeData();
    const secs = data && data.limits && data.limits.reclaim_grace_seconds;
    return (typeof secs === 'number' && secs > 0 ? secs : 120) * 1000;
  }

  function signal(to, payload) {
    if (socket && socket.readyState === 1) socket.send(frame('signal', { to, payload }));
  }

  /** Hand the lossy channel to the adapter once both exist. */
  function pairSnapshot(entry) {
    if (entry.adapter && entry.snapshot) entry.adapter.bindSnapshot(entry.snapshot);
  }

  /** Ordinary crew traffic, on whichever channel it arrived. */
  function deliver(entry, data) {
    if (entry.refused || !entry.admitted) return;
    const msg = decodeFrame(data);
    // A second handshake from an admitted peer is noise, not a re-vote.
    if (msg && msg.type === 'JoinHandshake') return;
    entry.adapter.emit('data', data);
  }

  /**
   * Wire a joiner's RELIABLE channel: the compatibility handshake, then the
   * page hand-off, then ordinary traffic.
   *
   * Hoisted out of `pc.ondatachannel` in #1113 so the WebSocket relay runs the
   * SAME admission gate rather than a second copy of it. That is not tidiness:
   * a fallback path with its own handshake would be a way into the host that
   * the stamp check does not cover, and the two would drift on the first change
   * to either. The relay's channels are DataChannel-shaped exactly so this
   * function cannot tell them apart (gui/rendezvous-relay.js).
   */
  function attachReliableChannel(id, entry, channel) {
    const adapter = connectionAdapter(id, channel, entry.pc, {
      // A page-initiated eviction is a refusal like any other: nothing this
      // peer sends afterwards, on EITHER channel, may reach the page again.
      onSever: () => {
        entry.refused = true;
        // For a WebRTC peer, closing the channels IS the eviction — the phone
        // sees `channel.onclose` and re-enters its reconnect loop. A RELAYED
        // peer's channels are local JavaScript objects, so the same close told
        // it nothing: it went on reading a status line that said connected and
        // sending commands the host dropped on the floor. Ask the service to
        // detach it, which sends it the `relay-closed` it already handles.
        if (entry.relay && socket && socket.readyState === 1) {
          socket.send(frame('relay-close', { to: id }));
        }
      },
      onLog,
    });
    entry.adapter = adapter;
    // The reliable channel's own close is the one true end of this peer:
    // reap the entry and its RTCPeerConnection right there, so a joiner
    // whose SIGNALLING died first (peer-left arrived, entry deliberately
    // retained) cannot leak a live ICE/DTLS agent until the host itself
    // loses its registration. The page's own close handler is identity-
    // guarded, so a later sweep emitting again is harmless.
    adapter.on('close', () => {
      try { entry.pc.close(); } catch { /* already gone */ }
      peers.delete(id);
    });
    pairSnapshot(entry);
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
      deliver(entry, ev.data);
    };
  }

  function peerState(id) {
    let entry = peers.get(id);
    if (entry) return entry;
    const pc = factories.peer(peerConfig());
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
      /** Set by relayPeerState below; null for an ordinary WebRTC joiner. */
      relay: null,
    };
    peers.set(id, entry);

    pc.onicecandidate = (e) => {
      if (e && e.candidate) signal(id, { candidate: e.candidate });
    };
    pc.oniceconnectionstatechange = () => onPeerIce(id, pc.iceConnectionState);

    pc.ondatachannel = (e) => {
      const channel = e.channel;
      if (channel.label === SNAPSHOT_CHANNEL) {
        entry.snapshot = channel;
        channel.onmessage = (ev) => deliver(entry, ev.data);
        pairSnapshot(entry);
        return;
      }
      if (channel.label !== RELIABLE_CHANNEL) return;
      attachReliableChannel(id, entry, channel);
    };
    return entry;
  }

  /**
   * A joiner the service is carrying for us, because it could not build a
   * direct link (issue #1113).
   *
   * From the page's side this is an ORDINARY connection: the same adapter, the
   * same compatibility handshake, the same Identify gate, the same per-token
   * snapshot routing. The only difference is what the channels are made of, and
   * `attachReliableChannel` above cannot tell.
   *
   * A peer with a LIVE path keeps it — an admitted, open DataChannel, or a
   * relay pair already built — because a joiner does not ask for the relay
   * while a DataChannel is working and two live paths to one crew member would
   * be a duplicate-delivery bug.
   *
   * Anything else is UPGRADED rather than returned, and that distinction is the
   * whole of this function. `peer-joined` creates an entry for every joiner the
   * moment it joins (see `handle()` below), and over a real ordered socket that
   * frame ALWAYS lands before `relay-peer`: the registry emits `peer-joined`
   * from `clientJoin`, one hop after `join`, while `relay-peer` needs a further
   * round trip (the joiner must receive `joined`, send `relay-open`, and be
   * answered). So by the time the service asks this host to carry a peer, that
   * peer already has a placeholder entry holding an RTCPeerConnection nothing
   * will ever negotiate. Returning it built no relay pair, ran no
   * `attachReliableChannel`, and left every subsequent `relay` frame to be
   * dropped by the `entry.relay` guard — the fallback rung was dead on the
   * deployed service while passing in a fixture whose dispatch is synchronous
   * and re-entrant enough to invert the two frames.
   */
  function relayPeerState(id, limits) {
    const existing = peers.get(id);
    if (existing && (existing.relay || isLiveAdmittedLink(existing))) return existing;
    if (existing) {
      // A placeholder from `peer-joined`, or a half-built WebRTC attempt this
      // joiner has given up on. Discard the unused peer connection rather than
      // leaving a live ICE/DTLS agent behind the relay entry that replaces it.
      try { existing.pc.close(); } catch { /* already gone */ }
      peers.delete(id);
    }

    const pair = createRelayChannelPair({
      send: (body) => {
        if (socket && socket.readyState === 1) socket.send(frame(body.type, body));
      },
      to: id,
      bufferedAmount: () => (socket && socket.bufferedAmount) || 0,
      limits: relayLimitsFromFrame(limits),
      onDegraded: ({ dropped }) => onPeerShedding(id, dropped, 'local'),
      onFailure: ({ reason }) => {
        // A reliable frame the relay cannot carry is a broken guarantee, not a
        // dropped frame. The pair closes itself locally, which reaches the
        // page through the adapter's ordinary `close` — but that is only THIS
        // host's half. Left there, the phone's mailbox at the service stays
        // open and the phone sits on a status line reading "connected" while
        // the host has already walked away — the same one-sided eviction
        // `onSever` above exists to prevent. Ask the service to detach it too,
        // before the local close reaches the page.
        if (socket && socket.readyState === 1) {
          socket.send(frame('relay-close', { to: id }));
        }
        onLog(`[rendezvous] relayed link to ${id} failed: ${reason}`);
      },
      onLog,
    });

    const entry = {
      // No ICE was negotiated, so there is no peer connection — but every
      // caller that holds one still gets an object rather than a null to guard.
      pc: relayPeerStub(() => pair.close()),
      adapter: null,
      snapshot: pair.snapshot,
      admitted: false,
      refused: false,
      pendingCandidates: [],
      relay: pair,
    };
    peers.set(id, entry);
    pair.snapshot.onmessage = (ev) => deliver(entry, ev.data);
    attachReliableChannel(id, entry, pair.reliable);
    pair.open();
    // The host operator's diagnostics row: this crew member is being carried by
    // the service, which is a degraded state worth seeing even though it works.
    onPeerIce(id, 'ws-relay');
    onLog(`[rendezvous] ${id} attached over the WebSocket relay`);
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

  /**
   * True for a joiner the compatibility handshake admitted whose reliable
   * channel is, right now, really open — the reliable channel's own
   * `readyState`, not this map's bookkeeping. This is THE test for "is this
   * an established link a signalling event may not touch": both
   * `dropUnadmittedPeers()` below and the `peer-left` handler apply it, so
   * the two mechanisms can only drift if this one definition does.
   */
  function isLiveAdmittedLink(entry) {
    // A RELAYED link is deliberately excluded, and this is the one place the
    // two planes are not independent. The rule those two mechanisms enforce is
    // "a signalling event may not touch a link that does not need signalling" —
    // and a relayed link IS the signalling socket. When the service goes, so
    // does the game path, so treating a relayed peer as live would leave the
    // page holding a connection nothing can reach.
    return !!(entry.admitted && entry.adapter && entry.adapter.open && !entry.relay);
  }

  /**
   * Discard the peers that existed only inside the dead REGISTRATION.
   *
   * A joiner still mid-signalling is one of them: its offer and answer were
   * crossing a socket that has gone, so no channel it is waiting on can ever
   * open and nobody is going to relay the rest of its ICE. An ADMITTED joiner
   * is NOT one of them. Its RTCPeerConnection and its two DataChannels are a
   * direct browser-to-browser link that owes the signalling plane nothing once
   * it is up — the phone cannot even observe that the record went away.
   * Dropping those here ended live missions over a Durable Object eviction, a
   * worker redeploy or a lazy TTL sweep: every admitted adapter got a `close`,
   * which in server.html is `wasm_player_disconnected(token)`.
   *
   * An admitted entry whose reliable channel has already gone is swept too. It
   * is pure bookkeeping — the page was told about that close when it happened —
   * and since this no longer clears the whole map, a host that lost the service
   * a few times would otherwise keep every dead peer it ever had: `peer-left`
   * can never arrive for one whose record is gone.
   *
   * That last sentence used to be strictly true; since the `peer-left` handler
   * below stopped tearing down a live admitted link itself, the ORDINARY reap
   * for such an entry is the reliable channel's own close handler (registered
   * in `pc.ondatachannel`), which closes the RTCPeerConnection and deletes the
   * map entry the moment the link genuinely ends. This sweep is the backstop
   * for entries that never reached that point.
   */
  function dropUnadmittedPeers() {
    for (const [id, entry] of [...peers]) {
      if (isLiveAdmittedLink(entry)) continue;
      if (entry.adapter) entry.adapter.emit('close');
      try { entry.pc.close(); } catch { /* already closed */ }
      peers.delete(id);
    }
  }

  function handle(msg) {
    switch (msg.type) {
      case 'ready':
        socket.send(frame('host-open', {
          namespace,
          // A browser host answers on both rungs; declaring it explicitly is
          // what lets a host that CANNOT (the native one, which has no WebRTC
          // at all) declare the truth in the same field rather than by
          // omission (issue #1113).
          transports: ['webrtc', 'ws-relay'],
          // issue #1115: present the reclaim secret from a PRIOR registration,
          // if this host is holding one, so the registry can hand back the
          // SAME code instead of minting a new one. Absent on this page's
          // very first registration — there is nothing yet to resume — and
          // harmless to send on every reconnect after that: a registry that
          // cannot honour it (grace expired, wrong secret, or simply does not
          // recognise the field) falls through to an ordinary fresh mint.
          ...(resumeToken ? { resume: resumeToken } : {}),
        }));
        break;
      case 'hosted':
        code = msg.code;
        retryAttempt = 0;
        // Re-registered — whether this is the reclaimed code or a fresh mint,
        // the loss streak is over, so the grace clock resets (issue #1115).
        serviceLostAt = null;
        // Only a registration/reclaim response carries `secret` — an
        // admission-state ACK reuses the same `hosted` type without one, and
        // must not clobber the held token with `undefined`.
        if (msg.code.secret) resumeToken = { suffix: msg.code.suffix, secret: msg.code.secret };
        onLog(`[rendezvous] issued ${code.namespace} code ${code.suffix}`);
        onCode(code);
        break;
      case 'peer-joined':
        peerState(msg.peer);
        break;
      case 'peer-left': {
        // registry.js's leave() sends this when THAT peer's own rendezvous
        // WebSocket dies — a Durable Object eviction, a worker redeploy, a
        // phone radio dropping the WS on a lock screen. It says nothing about
        // that peer's DataChannel. An admitted, still-open link is the exact
        // case `dropUnadmittedPeers()` above protects from a dead HOST
        // socket; a signalling event may not touch it here either, or a
        // healthy mid-mission player gets evicted (wasm_player_disconnected
        // + pc.close) while the joiner still thinks it's connected. Leave the
        // entry in the map — the reliable channel's own close handler
        // (registered in `pc.ondatachannel`) reaps it, closing the pc and
        // deleting the entry the moment the link genuinely ends, and that
        // same `onclose` is what reaches the page as a disconnect, exactly
        // as if this frame had never arrived.
        const entry = peers.get(msg.peer);
        if (entry && !isLiveAdmittedLink(entry)) {
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
      // ── The WebSocket game relay (issue #1113) ────────────────────────────
      case 'relay-peer':
        relayPeerState(msg.peer, msg.limits);
        break;
      case 'relay': {
        const entry = peers.get(msg.from);
        if (entry && entry.relay) entry.relay.deliver(msg);
        break;
      }
      case 'relay-peer-left': {
        // Unlike `peer-left`, this one really is the end of the link: a relayed
        // peer has no DataChannel to outlive its socket. Tear it down here so
        // the page runs its ordinary disconnect lifecycle.
        const entry = peers.get(msg.peer);
        if (entry && entry.relay) {
          if (entry.adapter) entry.adapter.emit('close');
          try { entry.pc.close(); } catch { /* already gone */ }
          peers.delete(msg.peer);
        }
        onPeerIce(msg.peer, 'closed');
        break;
      }
      case 'relay-degraded':
        // The service shedding this host's own outbound frames at the
        // (host,peer) mailbox before they reached that crew member. The host
        // operator is the one who can act on this direction of it, and until
        // #1113's review nothing here read the frame at all. `dropped` is the
        // mailbox's running total — a separate queue from `onDegraded` above,
        // so it is reported under its own `source` rather than folded in.
        onPeerShedding(msg.peer, msg.dropped || 0, 'service');
        break;
      case 'relay-closed':
        // The service closing OUR OWN relay mailbox — the reliable-queue
        // overflow in worker-rendezvous/src/relay.js. Every peer it was
        // carrying is now unreachable; say so rather than leaving the page
        // holding connections nothing can deliver to.
        onLog(`[rendezvous] the service closed our relay (${msg.reason || 'unknown'})`);
        for (const [id, entry] of [...peers]) {
          if (!entry.relay) continue;
          if (entry.adapter) entry.adapter.emit('close');
          try { entry.pc.close(); } catch { /* already gone */ }
          peers.delete(id);
          onPeerIce(id, 'closed');
        }
        onError(msg.reason || 'relay-overflow');
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
   * The signalling socket died: a network blip, a Durable Object hiccup, or the
   * service swept the record (the two look identical from here). There is no
   * second transport to fall back to any more, so a host that simply stopped
   * here would be unjoinable until someone reloaded the viewscreen — which is
   * why this re-registers instead of only reporting.
   *
   * **This is a SIGNALLING-plane event and it discards signalling state only**
   * — the socket, the code, the registration — plus the peers that were still
   * mid-signalling on it. Crew already playing keep playing: their
   * DataChannels are direct and need no service at all, and only their own
   * channel closing (or `close()` below) may ever reach the page as a
   * disconnect. What re-registration buys is a way in for NEW joiners.
   *
   * The replacement registration presents `resumeToken`, if this host is
   * holding one, and asks the registry for the SAME code back (issue #1115).
   * The record survives a socket loss for the authored
   * `[limits] reclaim_grace_seconds`, held un-hostable but otherwise intact —
   * see worker-rendezvous/src/registry.js's "code lifecycle" doc — so an
   * ordinary blip reconnects onto the letters already on screen and QR'd
   * across the room, rather than repainting new ones under everybody. Only
   * once that window has genuinely run out (or the DO instance itself was
   * evicted, taking the secret with it) does the registry mint a fresh
   * suffix instead, and `onCode` repaints exactly the same way either time —
   * the panel does not need to know which one happened.
   */
  function lostService(reason) {
    if (closed) return;
    // Stamp the START of this loss streak, not each failed retry within it —
    // the registry's grace clock runs from the ORIGINAL socket death, so
    // `resuming` measures elapsed time the same way (issue #1115).
    if (serviceLostAt === null) serviceLostAt = Date.now();
    // Detached and silenced BEFORE anything else: closing a socket fires its
    // own `close`, and a handler that could still see itself as the current
    // one would re-enter here and report the same loss twice.
    const dying = socket;
    socket = null;
    code = null;
    dropUnadmittedPeers();
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
    /**
     * True once this host is holding a reclaim token from a prior
     * registration — i.e. a re-registration will ASK for the same code back
     * rather than minting a fresh one outright. Read by server.html's
     * connection diagnostics to word the "service dropped, reconnecting"
     * line honestly (issue #1115): "reconnecting your code" when a reclaim
     * is actually in flight, the older "a new code is coming" only for a
     * first-ever registration that never got one to hold.
     *
     * Gated on the grace window: once a loss streak has outlasted
     * `reclaim_grace_seconds`, the registry has already dropped the held record
     * (its `expireStale`), so the pending reconnect will be issued a FRESH
     * suffix — no longer a reclaim, so this stops reporting one. A held token
     * with no active loss (an ordinary live host, or one mid-reconnect inside
     * the window) still reads `true`.
     */
    get resuming() {
      if (!resumeToken) return false;
      if (serviceLostAt !== null && Date.now() - serviceLostAt >= graceWindowMs()) {
        return false;
      }
      return true;
    },
    /** Open or close new-joiner admission without dropping the code. */
    setAdmission(state) {
      if (socket && socket.readyState === 1) socket.send(frame('host-admission', { state }));
    },
    /**
     * Explicit rotation (issue #1115 AC2/AC3): mint a BRAND NEW code for this
     * live record, in place. A no-op while there is no live socket to ask —
     * the caller (server.html) is expected to gate this on GamePhase before
     * ever calling it, since the registry has no notion of "a mission is
     * running" and cannot enforce that rule itself.
     */
    rotate() {
      if (socket && socket.readyState === 1) socket.send(frame('rotate', {}));
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
 * A link failure is retried on an exponential backoff whether or not the host
 * has accepted this build yet: the joiner re-resolves THE SAME code, re-offers,
 * re-sends the compatibility handshake and re-sends `Identify` with the same
 * session token — so the host restores the held station and pushes the current
 * projection, and the guest is never asked for the code a second time.
 * `retryNow()` short-circuits the wait for the page's "retry now" control.
 *
 * Once the host has accepted this build the loop is unbounded; BEFORE that it
 * runs {@link JOIN_ATTEMPTS_BEFORE_ENTRY} times, because a guest who has never
 * got in may simply be reading the wrong code. Either way, only a
 * refusal a retry cannot fix (see {@link isRetryableReason}) ends the loop
 * early, and the entry field comes back with its own sentence.
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
    levers = defaultTransportLevers(),
    factories = defaultFactories(),
    // ── The three options a SHIP HOST joiner needs (issue #1114) ────────────
    // A fleet member reaches a host over exactly this path — the same resolve,
    // offer, channel pair, compatibility handshake, backoff and diagnostics —
    // and differs only in what it is. It holds no station, so it does not
    // Identify; it speaks its own vocabulary, so its frames carry no
    // localisable string ids and it encodes them itself. Three narrow hooks
    // rather than a second copy of the whole dance.
    /**
     * Called instead of sending `Identify` once the host accepts this build.
     * `generation` identifies the concrete transport attempt, so consumers can
     * distinguish a real reconnect from a duplicate acceptance frame on the
     * still-live channel.
     */
    onAccepted = null,
    /** False for a peer whose frames are not ServerMessages. */
    localise = true,
  } = opts;

  /**
   * A joiner that never dialled: the code was refused before a socket was
   * opened. Every method the live surface advertises is present and inert, so a
   * caller holding a failed joiner gets a no-op rather than a TypeError —
   * including `sendFrame`, which `createFleetMember.update()` calls
   * unconditionally.
   */
  const refusedJoiner = (reason) => {
    onError(reason, reasonStringId(reason));
    return {
      failed: true,
      reason,
      get connected() { return false; },
      send() {},
      sendFrame() {},
      retryNow() {},
      close() {},
    };
  };

  const parsed = parseJoinCode(code, namespace, data);
  if (!parsed.ok) return refusedJoiner(parsed.reason);
  // A structured code names its OWN namespace, from the project GUID inside it
  // — `namespace` above is only the fallback a bare typed suffix is
  // composed under. So a full code pasted into the wrong field parses happily
  // and disagrees with the field it was typed into, and that disagreement is
  // exactly what "you typed the other kind of code" means. Refused here, before
  // a socket is opened, the same judgement server.html makes on a fleet code
  // arriving in this page's fragment.
  if (parsed.namespace !== namespace) return refusedJoiner('wrong-type');

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
  /**
   * Which rung of the transport ladder this joiner is on (issue #1113).
   *
   *   'direct'    a WebRTC offer — over any candidate, or only over a TURN
   *               relay one when `?forceRelay` pinned it (gui/transport-levers.js)
   *   'ws-relay'  the game's own frames over this same rendezvous socket
   *
   * It only ever escalates, never goes back. A network that refused WebRTC four
   * times running is not one to keep re-asking mid-mission, and a reload starts
   * the ladder again from the top for the case where the network changed.
   */
  let mode = levers.wsRelay === 'only' ? 'ws-relay' : 'direct';
  /** The relayed channel pair while `mode === 'ws-relay'`; null otherwise. */
  let relay = null;

  const linked = () => isChannelOpen(channel);
  /**
   * True when this joiner's game path IS the signalling socket.
   *
   * The two-planes rule at the top of this file — a signalling event may not
   * touch an established link — holds precisely because a DataChannel owes the
   * service nothing once it is up. A relayed link owes it everything, so while
   * relaying, `closed`, `error` and the socket's own death are link failures
   * rather than news.
   */
  const socketIsTheLink = () => mode === 'ws-relay';

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
    if (relay) { try { relay.close(); } catch { /* already gone */ } relay = null; }
    if (pc) { try { pc.close(); } catch { /* already gone */ } pc = null; }
    if (socket) {
      socket.onopen = null; socket.onmessage = null; socket.onerror = null; socket.onclose = null;
      try { socket.close(); } catch { /* already gone */ }
      socket = null;
    }
    pendingCandidates = [];
  }

  /**
   * This attempt failed. A terminal reason, or a retryable one that has used up
   * its pre-acceptance attempts, goes back to the page; anything else is the
   * reconnect loop.
   */
  function fail(gen, reason, detail) {
    if (closed || gen !== generation || gen === failedGeneration) return;
    failedGeneration = gen;
    teardown();
    // The direct ladder is spent and there is one rung left: let the service
    // carry the game itself (issue #1113). This runs BEFORE the ordinary retry
    // branch below, and it runs whether or not the host has accepted this build
    // — a phone that walked from a working network onto a hostile one is the
    // same problem as a phone that started on the hostile one, and the answer
    // is the same. `wsRelay: 'off'` (a pinned WebRTC mode) skips it, because a
    // fallback that fired would hide the failure the pin exists to expose.
    const ladderSpent = attemptIndex + 1 >= JOIN_ATTEMPTS_BEFORE_ENTRY;
    if (isRetryableReason(reason) && ladderSpent
        && mode === 'direct' && levers.wsRelay === 'auto') {
      mode = 'ws-relay';
      onLog('[rendezvous] direct link exhausted — falling back to the WebSocket relay');
      onDiag({ event: 'transport', transport: 'ws-relay', reason: 'direct-exhausted' });
      if (established) onStatus('disconnected');
      // A fresh ladder: the relay gets its own full budget of attempts, its own
      // backoff from the bottom, and its own attempt numbering in the
      // diagnostics readout — "attempt 5" on a path being tried for the first
      // time would be a lie about which rung had been given how long.
      scheduleRetry({ restart: true });
      return;
    }
    // A retryable failure is retried whether or not the host has accepted this
    // build yet. An ICE timeout, a dropped signalling socket and an unreachable
    // service are the same transient thing on a first join as on a reconnect,
    // and only by advancing `attemptIndex` here does `connectTimeoutMs`'s
    // 8/16/30 s ladder become reachable by the cellular guest it exists for.
    // Before acceptance the loop is BOUNDED (see JOIN_ATTEMPTS_BEFORE_ENTRY):
    // a guest who has never got in may be reading the wrong code, and
    // a silent backoff would never say so. After acceptance it is unbounded —
    // on whichever rung the escalation above left this joiner on.
    if (isRetryableReason(reason)
        && (established || attemptIndex + 1 < JOIN_ATTEMPTS_BEFORE_ENTRY)) {
      onLog(`[rendezvous] link lost (${reason}) — retrying`);
      // 'disconnected' is the page's "Disconnected — reconnecting…" treatment,
      // and it belongs to a link that WAS up. A guest still trying to get in
      // for the first time stays on 'connecting' — attempt() re-reports it on
      // the way out of the backoff — rather than being told a connection they
      // never had has dropped.
      if (established) onStatus('disconnected');
      scheduleRetry();
      return;
    }
    // The loop has stopped. Report the terminal state whether or not this link
    // was ever accepted: 'disconnected' renders as "reconnecting…" with a
    // retry control under it, and nothing is retrying any more.
    onError(reason, detail);
    onStatus('error');
  }

  /**
   * `restart: true` begins a NEW ladder rather than continuing this one — the
   * transport escalation in `fail()` is its only caller. The next attempt is
   * then attempt 1 with the bottom rung of both schedules, which is what a path
   * that has not been tried yet is entitled to.
   */
  function scheduleRetry({ restart = false } = {}) {
    if (closed || retryTimer) return;
    const delay = nextBackoffDelay(restart ? 0 : attemptIndex);
    attemptIndex = restart ? 0 : attemptIndex + 1;
    onLog(`[rendezvous] retrying in ${delay}ms (attempt ${attemptIndex + 1})`);
    retryTimer = setTimeout(() => { retryTimer = null; attempt(); }, delay);
  }

  function signal(payload) {
    if (socket && socket.readyState === 1) socket.send(frame('signal', { payload }));
  }

  /**
   * Wire this attempt's two channels: the compatibility handshake on open, the
   * reconnect loop on close, and ordinary ServerMessage traffic in between.
   *
   * Hoisted out of `offer()` in #1113 so the relayed pair
   * (gui/rendezvous-relay.js) runs through exactly the same handshake, the same
   * `established`/`attemptIndex` bookkeeping and the same `deliver` ingress. A
   * second copy of this for the fallback path would be a second place for the
   * stamp handshake to drift.
   */
  function wireChannels(gen) {
    snapshot.onmessage = (e) => deliver(gen, e.data);
    snapshot.onopen = () => onLog('[rendezvous] snapshot channel open');

    channel.onopen = () => {
      if (gen !== generation) return;
      if (connectTimer) { clearTimeout(connectTimer); connectTimer = null; }
      onDiag({ event: 'open' });
      // Which candidate pair ICE actually chose, and over which relay if one
      // carried it (issue #1113). "candidates: host, srflx, relay" says what
      // was OFFERED; on a hotspot the difference between a server-reflexive
      // pair and a relayed one is the difference between a working network and
      // a working TURN worker, and only the selected pair answers that.
      // Skipped on the relayed path, which negotiated no ICE at all.
      if (pc) {
        readSelectedPair(pc).then((pair) => {
          if (pair && gen === generation) onDiag({ event: 'selected-pair', pair });
        });
      }
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
        if (onAccepted) {
          // A ship host announces itself in its own vocabulary instead — and
          // on every reconnect, for the same reason the crew path re-sends
          // Identify: the far end restores this peer's slot from what it says
          // here, not from a memory of a connection that has gone.
          onAccepted({ generation: gen });
          return;
        }
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
  }

  /**
   * Ask the service to carry this joiner's game frames (issue #1113). Called
   * instead of `offer()` once the direct ladder is spent, or straight away when
   * `?transport=ws-relay` pinned it.
   */
  function relayAttach(gen, readyFrame) {
    if (gen !== generation) return;
    relay = createRelayChannelPair({
      send: (body) => {
        if (socket && socket.readyState === 1) socket.send(frame(body.type, body));
      },
      bufferedAmount: () => (socket && socket.bufferedAmount) || 0,
      limits: relayLimitsFromFrame(readyFrame && readyFrame.limits),
      onDegraded: ({ dropped }) => onDiag({ event: 'relay-degraded', dropped, from: 'client' }),
      onLog,
    });
    channel = relay.reliable;
    snapshot = relay.snapshot;
    wireChannels(gen);
    // 'open' the moment the service has confirmed the attachment: on this path
    // that IS "the far end can be reached", exactly as a DataChannel's open is.
    relay.open();
  }

  async function offer(gen) {
    // Both halves of the ICE lever, and they pull opposite ways on purpose
    // (gui/transport-levers.js): `iceTransportPolicy: 'relay'` carries
    // `?forceRelay` — the browser discards host and server-reflexive
    // candidates, so any pair that forms is over a TURN allocation — while
    // `useIceServers: false` carries `?transport=direct`, withholding the
    // server list entirely so there is no allocation to make and no relay
    // candidate to gather. `iceTransportPolicy` has no 'no-relay' value, so
    // that second pin can only be spelled this way.
    pc = factories.peer({
      iceServers: levers.useIceServers === false ? [] : iceServers,
      iceTransportPolicy: levers.iceTransportPolicy,
    });
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
    wireChannels(gen);

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
    // A host-mesh peer opts out: its frames carry no string ids, and walking
    // them would be a resolver looking for ids in another protocol's data.
    onData(localise ? localiseTree(msg) : msg);
  }

  function handle(gen, msg) {
    switch (msg.type) {
      case 'ready':
        if (!established) onStatus('connecting');
        onDiag({ event: 'signaling', state: 'open' });
        // The joiner's own stamp travels in the in-band JoinHandshake, to the
        // host that decides on it — never through the service, which has no
        // say and would only be relaying dead metadata.
        //
        // The NAMESPACE does travel, because it is a fact about the request
        // rather than about either build: it names the typed FIELD this code
        // came out of, which is the only thing that makes "that is the other
        // kind of code" answerable now that both namespaces are joinable
        // (worker-rendezvous/src/registry.js, issue #1114).
        //
        // `namespace`, never `parsed.namespace`. The parsed value is a fact
        // about the CODE — for a structured one it is read out of the project
        // GUID the code itself carries — so sending it would have the asker
        // echo the record back at the service, which then always agrees and
        // can never answer `wrong-type`. The check above already refuses the
        // disagreeing case locally; this is the same fact stated to the one
        // party that indexes on it.
        socket.send(frame('join', { code: parsed.full, namespace }));
        break;
      case 'joined':
        // The host has said what it can answer on. A host with no WebRTC — the
        // native one — must not be dialled for ninety seconds first, and a
        // joiner pinned to a WebRTC mode against such a host is told so rather
        // than left timing out against a rung that does not exist.
        if (Array.isArray(msg.transports) && !msg.transports.includes('webrtc')) {
          if (levers.wsRelay === 'off') {
            onLog('[rendezvous] this host answers only on the relay, and the transport is pinned');
            fail(gen, 'not-joinable');
            break;
          }
          if (mode !== 'ws-relay') {
            mode = 'ws-relay';
            onLog('[rendezvous] host answers only on the relay — skipping the direct ladder');
            onDiag({ event: 'transport', transport: 'ws-relay', reason: 'host-has-no-webrtc' });
          }
        }
        if (mode === 'ws-relay') {
          onLog(`[rendezvous] resolved ${parsed.suffix}; asking the service to relay`);
          socket.send(frame('relay-open', {}));
          break;
        }
        onLog(`[rendezvous] resolved ${parsed.suffix}; offering`);
        offer(gen).catch((e) => fail(gen, 'unreachable', String(e && e.message)));
        break;
      case 'signal':
        onSignal(gen, msg.payload).catch((e) => fail(gen, 'unreachable', String(e && e.message)));
        break;
      // ── The WebSocket game relay (issue #1113) ────────────────────────────
      case 'relay-ready':
        relayAttach(gen, msg);
        break;
      case 'relay':
        if (relay) relay.deliver(msg);
        break;
      case 'relay-closed':
        // The service has stopped carrying this link. Unlike a `closed` frame
        // against a DataChannel, this really is the end of the game path.
        onLog(`[rendezvous] relay closed (${msg.reason || 'unknown'})`);
        fail(gen, isRetryableReason(msg.reason) ? 'unreachable' : msg.reason);
        break;
      case 'relay-degraded':
        onDiag({ event: 'relay-degraded', dropped: msg.dropped, from: 'service' });
        break;
      // Signalling-plane news. Before the direct channels are up these are
      // answers, not blips — the record is gone, or the service refused the
      // request — so the attempt is torn down as well as reported, and a dead
      // attempt can never fire a page callback later.
      //
      // Once `linked()` is true they are INFORMATION. A `closed` frame is a
      // statement about the rendezvous RECORD (its host's socket dropped, or
      // the lazy TTL sweep took it — worker-rendezvous/src/registry.js's
      // `dropRecord`), and an `error` frame is usually a race about a peer id
      // the service no longer holds. Neither says anything about a DataChannel
      // that is already carrying the game, and acting on them killed healthy
      // sessions: `host-gone` is terminal, so the joiner stopped for good with
      // the link still up. Only the channel's own close drives the reconnect
      // loop from here — and if the link then genuinely goes, the next resolve
      // gets the honest answer about the code.
      //
      // `socketIsTheLink()` is the exception #1113 adds: while the SERVICE is
      // carrying the game there is no second plane to be independent of, so
      // these frames are answers again rather than news.
      case 'closed':
        if (linked() && !socketIsTheLink()) {
          onLog(`[rendezvous] record closed (${msg.reason || 'host-gone'}) — the direct link is up, playing on`);
          break;
        }
        fail(gen, msg.reason || 'host-gone');
        break;
      case 'error':
        if (linked() && !socketIsTheLink()) {
          onLog(`[rendezvous] service error (${msg.reason}) — the direct link is up, playing on`);
          break;
        }
        fail(gen, msg.reason, msg.detail);
        break;
      default:
        break;
    }
  }

  async function onSignal(gen, payload) {
    if (gen !== generation || !pc) return;
    // This handler crosses browser promises.  Keep the concrete attempt's PC
    // and candidate queue: teardown() replaces both globals, so an old SDP
    // completion must never resume against the retry which replaced it.
    const mine = pc;
    const mineCandidates = pendingCandidates;
    if (payload && payload.sdp) {
      await mine.setRemoteDescription(payload.sdp);
      if (gen !== generation || pc !== mine || pendingCandidates !== mineCandidates) return;
      for (const c of mineCandidates.splice(0)) await mine.addIceCandidate(c);
    } else if (payload && payload.candidate) {
      if (mine.remoteDescription) await mine.addIceCandidate(payload.candidate);
      else mineCandidates.push(payload.candidate);
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
    // Which rung of the ladder this attempt is on, and whether a lever pinned
    // it there. Reported on EVERY attempt rather than only on a change, so the
    // readout always names a transport instead of leaving the first rung to be
    // inferred from silence.
    onDiag({ event: 'transport', transport: mode, pinned: levers.pinned, mode: levers.mode });
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
      if (closed || gen !== generation || socket !== mine) return;
      // A live DIRECT link does not care that its signalling socket went; a
      // relayed one has just lost the wire the game was on (issue #1113).
      if (linked() && !socketIsTheLink()) return;
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
      if (!isChannelOpen(channel)) return;
      // Same ceiling as the host's side of the adapter: the SDP-negotiated
      // `max-message-size` (262144 bytes between two Chromiums) on a raw
      // RTCDataChannel with no chunking under it. A ClientMessage is a command
      // with a handful of fields, so nothing here is remotely near it — but an
      // uncaught throw would propagate into whichever console control called
      // `send`, and a dropped command is a better outcome than a broken UI
      // handler.
      try {
        channel.send(json);
      } catch (e) {
        onLog(`[rendezvous] send failed (${type}) — dropping this command: ${e && e.message}`);
      }
    },
    /**
     * Send one already-encoded frame on the reliable channel (issue #1114).
     *
     * `send` above builds the crew protocol's `{type, data}` shape. A ship host
     * speaks a different vocabulary and encodes it in its own module
     * (`gui/host-mesh.js`), so it needs the channel and not the wrapper —
     * having this function learn a second envelope would put the host protocol
     * in the crew transport, which is the thing the two protocols being
     * separate is for.
     */
    sendFrame(json) {
      if (!isChannelOpen(channel)) return;
      try {
        channel.send(json);
      } catch (e) {
        onLog(`[rendezvous] frame send failed — dropping it: ${e && e.message}`);
      }
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
    JOIN_ATTEMPTS_BEFORE_ENTRY,
    isRetryableReason,
    rendezvousBaseFromLocation,
    rendezvousBaseForOrigin,
    joinRouteFromLocation,
    socketUrl,
    joinUrlForCode,
    defaultFactories,
    createRendezvousHost,
    createRendezvousJoiner,
  };
}
