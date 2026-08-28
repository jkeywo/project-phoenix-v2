/**
 * worker-rendezvous/src/registry.js — the Phoenix rendezvous protocol, as a
 * pure state machine (issue #1111).
 *
 * This is the whole service. `src/index.js` is only an adapter: it terminates
 * the WebSocket, checks the request Origin, and pumps frames in and out of the
 * object this module returns. Typed code lookup, admission state, signalling
 * relay, presence and code lifecycle all live in HERE, behind one versioned
 * frame vocabulary, so #1114 (server-code joining) and #1115 (code rotation)
 * extend a module rather than rewriting a worker.
 *
 * Keeping it transport-free is also what makes it testable: the rendezvous
 * contract tests in tests/client/rendezvous-registry.test.js drive this object
 * directly, with no wrangler, no miniflare and no sockets.
 *
 * ── The frame vocabulary (v1) ───────────────────────────────────────────────
 *
 * Every frame carries `v`. A frame with another `v` is refused rather than
 * guessed at, which is what lets the vocabulary grow without a flag day.
 *
 * host → service           service → host              service → client
 *   host-open                hosted                      resolved
 *   host-admission           peer-joined                 joined
 *   rotate                   peer-left                   signal
 *   signal                   signal                      closed
 *   host-close               error                       error
 *   relay                    relay-peer                  relay-ready
 *                            relay-peer-left             relay-closed
 * client → service           relay                       relay
 *   resolve | join |                                     relay-degraded
 *   signal | leave |
 *   relay-open | relay |
 *   relay-close
 *
 * The four `relay*` verbs are issue #1113's, and they are a fallback rather
 * than a second route: they carry OPAQUE game payloads over the same socket
 * when a joiner's direct WebRTC ladder is exhausted. Everything about how they
 * are bounded, and the load-bearing rule that this service may never learn what
 * is inside a payload, lives in src/relay.js. `rotate` is issue #1115's, sitting
 * in the host column with the reclaim `resume` on `host-open` (both below).
 *
 * `resolve` and `join` carry an optional `namespace` naming which typed field
 * the code was entered into — the crew field on a phone, the fleet field on a
 * ship host (issue #1114). Absent means `client`, so a frame written before
 * that issue means exactly what it always did. It is what keeps "you typed the
 * other kind of code" answerable now that BOTH namespaces are joinable: a fleet
 * code is a perfectly good record, just not one a phone may attach to.
 *
 * `host-open` carries an optional `resume: { suffix, secret }` (issue #1115):
 * a host asking for the SAME suffix it held before its signalling socket died,
 * proven with the one-time secret its own earlier `hosted` frame carried. See
 * "code lifecycle" below. `rotate` is the operator's explicit lever — mint this
 * LIVE record's host a brand-new suffix in place, dropping the old one for
 * good immediately (issue #1115 AC2/AC3). Neither needs a `v` bump: an older
 * host simply never sends them, and the registry answers exactly as it always
 * has when it does not see them.
 *
 * Failures are always an `error` frame naming the request and one stable
 * machine `reason`; the phone maps that reason to a strings.csv id through
 * gui/join-code.js's `reasonStringId`, so the service never ships prose.
 *
 * ── Code lifecycle (issue #1115) ────────────────────────────────────────────
 *
 * A record no longer dies the instant its host's SOCKET does. `hostOpen`
 * mints every record a one-time reclaim `secret` (32 lowercase hex
 * characters, the session-token shape — never sent to a joiner, never stored
 * anywhere but this record and the host's own transport-plane memory). An
 * EXPLICIT `host-close` still drops the record for real and immediately, the
 * same as always — a deliberate teardown has nothing to reclaim. A socket
 * merely dying — `disconnect()`, with no `host-close` frame ever seen — instead
 * holds the record in GRACE (`enterGrace`) for the authored
 * `reclaim_grace_seconds`: no live host to relay signalling to (a `join`
 * against it answers the retryable `unreachable`, same as any other transient
 * link failure), but the suffix, its secret, its admission state and its
 * presence all survive untouched. `hostOpen`'s `resume` handling is the only
 * way back in before the deadline; a secret that does not match burns the
 * grace-held record outright (denying further guesses) and falls through to
 * an ordinary fresh mint, exactly as if `resume` had never been sent.
 *
 * This is still bounded by the Durable Object's own lifetime, and deliberately
 * so: the secret lives in the SAME in-memory record as everything else, so an
 * instance eviction (rather than an ordinary socket drop) loses it along with
 * the record — there is no persistent store backing this, and #1115 does not
 * add one.
 *
 * ── What v1 deliberately does NOT do ────────────────────────────────────────
 *
 * All state here is IN MEMORY, in one Durable Object instance: records die
 * with the instance, and nothing is written to storage. A code therefore
 * cannot survive a worker redeploy or a Durable Object eviction, and it cannot
 * be rebound to a REPLACEMENT host — only reclaimed by the one that minted it,
 * within the grace window above. The `record_ttl_seconds` sweep below is a
 * SEPARATE, much longer backstop: an idle timeout measured from `lastSeen`,
 * refreshed by every inbound frame from the record's own host, not from when
 * the record was created — so a host that has been live for days is never
 * swept merely for being old. It catches the case where the socket dies AND
 * the host never comes back to reclaim within its grace window either — the
 * record is still held (now un-hostable) until this sweep finally gives up on
 * it.
 *
 * The bounds in the authored `[limits]` table are the other half of that
 * posture. This is an unauthenticated public endpoint whose codes are private
 * and whose suffix space is small (25^5 ≈ 9.8M), so an unbounded socket could
 * walk the whole namespace, hold a record open forever, or store whatever it
 * liked on one. None of them is a rate limiter across sockets — that is
 * Cloudflare's own to apply at the edge, and it is on the delivery checklist
 * rather than in here.
 */

import {
  NAMESPACE_CLIENT,
  NAMESPACE_SERVER,
  parseJoinCode,
  namespaceOf,
  projectGuidFor,
  versionGuid,
  mintSuffix,
  composeJoinCode,
  checkJoinCodeFormat,
} from '../../gui/join-code.js';
import { RENDEZVOUS_PROTOCOL } from '../../gui/rendezvous-protocol.js';
import { createRelayHub, isRelayClass } from './relay.js';

/**
 * Frame-vocabulary revision, from the one module both ends of the join path
 * import. Re-exported because src/index.js and the contract tests read it from
 * the registry — the service's own answer for "what do I speak".
 */
export { RENDEZVOUS_PROTOCOL };

/** Roles a socket may open with. The adapter derives these from the path. */
export const ROLE_HOST = 'host';
export const ROLE_CLIENT = 'client';

const recordKey = (project, version, suffix) =>
  `${String(project).toLowerCase()}|${String(version).toLowerCase()}|${suffix}`;

/**
 * The shape a release identifier must have to become part of a record key: a
 * canonical GUID, which is what `assets/join/join-codes.toml` authors and what
 * every real host sends. Deliberately strict — this is the one host-supplied
 * value the registry stores and indexes on.
 */
const RELEASE_GUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The transports a host may claim. Anything else is dropped, not relayed. */
const TRANSPORTS = ['webrtc', 'ws-relay'];

/**
 * What a host said it can answer on, reduced to the names this vocabulary
 * knows. An absent, empty or unrecognised claim means BOTH — the browser
 * host's own answer, and what every host built before this field existed
 * meant by saying nothing.
 */
function sanitiseTransports(claimed) {
  if (!Array.isArray(claimed)) return [...TRANSPORTS];
  const kept = TRANSPORTS.filter((name) => claimed.includes(name));
  return kept.length ? kept : [...TRANSPORTS];
}

function defaultRandomInt(n) {
  // crypto is present in Workers, in browsers and in Node 20+.
  const buf = new Uint32Array(1);
  crypto.getRandomValues(buf);
  return buf[0] % n;
}

/**
 * A fresh reclaim secret for one record (issue #1115): 32 lowercase hex
 * characters, the same shape AGENTS.md rule 2 uses for a session token.
 * Deliberately NOT drawn through the injectable `randomInt` the suffix mint
 * uses — that seam exists so tests can SCRIPT an exact, often deliberately
 * weak or repeating, sequence of suffix draws (`rendezvous-registry.test.js`
 * scripts collisions, denied words, exhaustion), and a security-sensitive
 * secret sharing it would either inherit that weakness or silently shift
 * every scripted suffix sequence by 32 draws the moment this function starts
 * being called. `crypto` directly, exactly like {@link defaultRandomInt}'s
 * own fallback, keeps the two fully independent.
 *
 * Transport-plane only, same doctrine as the join code itself: never touches
 * simulation state or a snapshot, never sent to a joiner, handed to the
 * record's own host exactly once — in the `hosted` frame that mints or
 * revives it — and never repeated on a later `hosted` (an admission ACK's
 * reuse of {@link codeOf}, for instance) because the host is expected to
 * remember it from that first frame.
 */
function mintSecret() {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Build a rendezvous registry.
 *
 * @param {object} opts
 * @param {object} opts.data authored table from assets/join/join-codes.json
 * @param {(n:number)=>number} [opts.randomInt] injectable draw, for tests
 * @param {()=>number} [opts.now] injectable clock, for tests
 * @param {string[]} [opts.joinableNamespaces] which namespaces admit a join.
 *   #1111 shipped `['client']` and #1114 added `'server'` — one entry, exactly
 *   as that comment promised, because the lookup was already typed. The
 *   parameter stays because "a namespace that exists so wrong-type is
 *   answerable but which nothing may join" is a state a future namespace can
 *   be in again; it is not a switch for turning fleets off.
 */
export function createRegistry({
  data,
  randomInt = defaultRandomInt,
  now = () => Date.now(),
  joinableNamespaces = [NAMESPACE_CLIENT, NAMESPACE_SERVER],
} = {}) {
  if (!data) throw new Error('rendezvous: no join-code format data');
  // The Worker bundles this table at build time; a mismatch means the service
  // and the phones are reading different schemas, which is a deploy fault, not
  // a request to serve badly.
  checkJoinCodeFormat(data);

  // Authored bounds, with parse-time defaults so an older table still loads.
  const limits = data.limits || {};
  const ttlMs = (limits.record_ttl_seconds || 43200) * 1000;
  const maxLookups = limits.max_lookups_per_connection || 60;
  const maxPeers = limits.max_peers_per_record || 32;
  const maxCodeLength = limits.max_code_length || 160;
  // issue #1115: how long a record survives its host's SOCKET dying before
  // it is dropped for good — see "code lifecycle" above.
  const reclaimGraceMs = (limits.reclaim_grace_seconds || 120) * 1000;

  /**
   * The WebSocket game relay (issue #1113), in its own module because its
   * bounds and its resource profile are nothing like the join protocol's. A
   * record only ever grows one of these lazily — nothing is allocated for a
   * mission whose crew all got a direct link.
   */
  const relay = createRelayHub({ limits });

  /** @type {Map<string, object>} recordKey → host record */
  const records = new Map();
  /** @type {Map<string, object>} connection id → {role, key, lookups} */
  const conns = new Map();

  const joinable = new Set(joinableNamespaces);

  const out = (to, frame) => ({ to, frame: { v: RENDEZVOUS_PROTOCOL, ...frame } });
  const fail = (to, request, reason, detail) =>
    out(to, { type: 'error', request, reason, ...(detail ? { detail } : {}) });
  /**
   * A refusal the adapter should also CLOSE the socket after sending. The
   * registry holds no sockets, so "refuse the socket" can only be an
   * instruction — src/index.js honours the flag, and a transport that ignores
   * it still gets the refusal because the connection stays capped.
   */
  const cut = (to, request, reason) => ({ ...fail(to, request, reason), close: true });

  const recordFor = (connId) => {
    const c = conns.get(connId);
    return c && c.key ? records.get(c.key) || null : null;
  };

  /**
   * Typed lookup. Returns `{ok:true, record}` or `{ok:false, reason}` with one
   * of the three failures the PRD demands kept apart: `unknown`, `wrong-type`
   * and `version-mismatch`. Nothing beyond the class of failure is revealed.
   *
   * `expected` is the namespace the ASKER is asking within — the crew field on
   * a phone, the fleet field on a ship host. Since #1114 both namespaces are
   * joinable, so "you typed the other kind of code" can no longer be answered
   * by the record's own type alone: a fleet code IS joinable, just not by the
   * phone that typed it into the crew field. This parameter is what keeps that
   * refusal typed instead of admitting a phone to a fleet record.
   */
  function lookup(project, version, suffix, expected) {
    const exact = records.get(recordKey(project, version, suffix));
    if (exact) {
      const ns = namespaceOf(exact.project, data);
      if (!joinable.has(ns)) return { ok: false, reason: 'wrong-type' };
      if (expected && ns !== expected) return { ok: false, reason: 'wrong-type' };
      return { ok: true, record: exact };
    }
    // Same release, the other typed namespace: the operator typed a server
    // code into the crew field (or the reverse).
    for (const ns of [NAMESPACE_CLIENT, NAMESPACE_SERVER]) {
      const other = projectGuidFor(ns, data);
      if (!other || other.toLowerCase() === String(project).toLowerCase()) continue;
      if (records.has(recordKey(other, version, suffix))) {
        return { ok: false, reason: 'wrong-type' };
      }
    }
    // Right namespace, another active release.
    for (const rec of records.values()) {
      if (rec.suffix !== suffix) continue;
      if (rec.project.toLowerCase() === String(project).toLowerCase()) {
        return { ok: false, reason: 'version-mismatch' };
      }
    }
    // Registered under some other namespace AND release.
    for (const rec of records.values()) {
      if (rec.suffix === suffix) return { ok: false, reason: 'wrong-type' };
    }
    return { ok: false, reason: 'unknown' };
  }

  /**
   * Which namespace a `resolve`/`join` frame is asking within.
   *
   * Absent, misspelled or anything but `server` means the crew namespace, which
   * is exactly #1111's behaviour — the field is additive, and a phone built
   * before #1114 keeps resolving crew codes without sending it.
   */
  const askedNamespace = (frame) =>
    frame && frame.namespace === NAMESPACE_SERVER ? NAMESPACE_SERVER : NAMESPACE_CLIENT;

  function resolveRequest(code, expected) {
    // Bound before parsing: a code is a bounded identifier, and a megabyte of
    // "code" is a request to spend the Durable Object's CPU, not to join.
    if (typeof code !== 'string' || code.length > maxCodeLength) {
      return { ok: false, reason: 'malformed' };
    }
    // The asked namespace is also the FALLBACK the parse composes a bare
    // five-letter suffix around: five letters typed into the fleet field are a
    // server code, and composing them under the crew project would resolve the
    // wrong record entirely rather than refuse.
    const parsed = parseJoinCode(code, expected, data);
    if (!parsed.ok) return { ok: false, reason: parsed.reason };
    return lookup(parsed.project, parsed.version, parsed.suffix, expected);
  }

  /**
   * Charge one lookup to a connection. Returns a refusal frame once the
   * connection is past its authored cap, and keeps returning one afterwards:
   * the socket is done regardless of whether the adapter closed it.
   */
  function chargeLookup(conn, connId, request) {
    conn.lookups = (conn.lookups || 0) + 1;
    return conn.lookups > maxLookups ? [cut(connId, request, 'too-many-attempts')] : null;
  }

  /**
   * Drop records that have gone quiet for longer than the TTL — measured from
   * `lastSeen`, not `createdAt`, so a host that has been live and talking for
   * days is never mistaken for an orphan just because it is old. A host
   * record normally dies with its socket; this is the sweep for the case
   * where the close never arrived (an evicted instance, a half-open socket),
   * so a code cannot be held forever by a host that is not there. Runs lazily
   * on every inbound frame — there is no alarm to schedule in a registry that
   * holds no storage.
   */
  function expireStale() {
    const cutoff = now() - ttlMs;
    const graceCutoff = now();
    let frames = [];
    for (const record of [...records.values()]) {
      // A grace-held record (issue #1115) has its own, much shorter deadline —
      // checked first and independently of `lastSeen`, which stopped advancing
      // the moment the host's socket died and would otherwise leave it looking
      // fresh for the whole idle TTL below.
      if (record.graceUntil !== null && record.graceUntil <= graceCutoff) {
        frames = frames.concat(dropRecord(record, 'host-gone'));
        continue;
      }
      if (record.lastSeen > cutoff) continue;
      frames = frames.concat(dropRecord(record, 'host-gone'));
    }
    return frames;
  }

  // ── Host requests ────────────────────────────────────────────────────────

  function hostOpen(connId, frame) {
    const conn = conns.get(connId);
    if (!conn || conn.role !== ROLE_HOST) return [fail(connId, 'host-open', 'forbidden-role')];
    if (conn.key) return [fail(connId, 'host-open', 'already-hosting')];

    const namespace = frame.namespace || NAMESPACE_CLIENT;
    const project = projectGuidFor(namespace, data);
    if (!project) return [fail(connId, 'host-open', 'wrong-type')];
    // The one field a host chooses that ends up in a record key, so the one
    // that needs a shape: a release identifier, not an essay. Everything
    // downstream lower-cases and concatenates it.
    const version = frame.version === undefined ? versionGuid(data) : frame.version;
    if (!RELEASE_GUID.test(String(version))) {
      return [fail(connId, 'host-open', 'malformed')];
    }

    // ── Reclaim (issue #1115) ──────────────────────────────────────────────
    // A `resume` descriptor is this host asking for the SAME suffix it held
    // before its signalling socket died — see the module doc's "code
    // lifecycle" section. Anything about it that does not check out (wrong
    // shape, no matching record, the record is not — or no longer — grace-
    // held) falls straight through to an ordinary fresh mint below, exactly
    // as if `resume` had never been sent. A secret that DOES fail to match a
    // genuinely grace-held record is the one case that does not merely fall
    // through: that record is burned outright, so a wrong guess cannot be
    // retried against the same suffix, and this same host presenting the
    // same wrong secret again gets a fresh code exactly as a first-time host
    // would.
    const resume = frame.resume;
    if (resume && typeof resume === 'object'
        && typeof resume.suffix === 'string' && typeof resume.secret === 'string') {
      const held = records.get(recordKey(project, version, resume.suffix));
      if (held && held.namespace === namespace
          && held.graceUntil !== null && held.graceUntil > now()) {
        if (held.secret === resume.secret) {
          held.host = connId;
          held.graceUntil = null;
          held.lastSeen = now();
          conn.key = held.key;
          return [
            out(connId, {
              type: 'hosted',
              code: { ...codeOf(held), secret: held.secret },
              admission: held.admission,
            }),
          ];
        }
        records.delete(held.key);
      }
    }

    const minted = mintSuffix(
      data,
      (s) => records.has(recordKey(project, version, s)),
      randomInt,
    );
    if (!minted.ok) return [fail(connId, 'host-open', minted.reason)];

    const key = recordKey(project, version, minted.suffix);
    const record = {
      key,
      project,
      version,
      suffix: minted.suffix,
      namespace,
      host: connId,
      // Which transports this host can actually answer on (issue #1113),
      // relayed verbatim to every joiner so it does not have to spend the whole
      // WebRTC ladder discovering that the host has no WebRTC. A NATIVE host
      // has none — it is a Rust process with a WebSocket, not a browser — so
      // its crew must go straight to the relay. A browser host offers both and
      // omitting the field means exactly that, so an older host still works.
      transports: sanitiseTransports(frame.transports),
      // No host delivery stamp is stored or relayed. The authoritative
      // protocol/content check is the host's own, in-band over the DataChannel
      // (delivery::check_join_stamp), and a copy here was read by nothing —
      // dead surface #1114 and #1115 would have had to keep maintaining.
      admission: 'open',
      peers: new Set(),
      createdAt: now(),
      // The TTL sweep expires on THIS, refreshed by every inbound frame from
      // `host` below (see `receive()`) — not on `createdAt` — so a record
      // stays alive for as long as its host keeps talking, however long that
      // is, and only a host that has genuinely gone quiet gets swept.
      lastSeen: now(),
      // issue #1115 — see the module doc's "code lifecycle" section.
      secret: mintSecret(),
      graceUntil: null,
    };
    records.set(key, record);
    conn.key = key;

    return [
      out(connId, {
        type: 'hosted',
        code: { ...codeOf(record), secret: record.secret },
        admission: record.admission,
      }),
    ];
  }

  function hostAdmission(connId, frame) {
    const record = recordFor(connId);
    if (!record || record.host !== connId) {
      return [fail(connId, 'host-admission', 'not-hosting')];
    }
    const state = frame.state === 'closed' ? 'closed' : 'open';
    record.admission = state;
    return [out(connId, { type: 'hosted', code: codeOf(record), admission: state })];
  }

  const codeOf = (record) => ({
    full: composeJoinCode(record),
    suffix: record.suffix,
    project: record.project,
    version: record.version,
    namespace: record.namespace,
  });

  function hostClose(connId) {
    const record = recordFor(connId);
    if (!record || record.host !== connId) return [];
    // A DELIBERATE close, unlike a socket merely dying (see `enterGrace`) —
    // there is nothing here to reclaim, so this drops the record for real and
    // immediately, exactly as it always has.
    return dropRecord(record, 'host-gone');
  }

  /**
   * Explicit rotation (issue #1115 AC2/AC3): mint this record's host a BRAND
   * NEW suffix, in place. The old record dies for good, immediately — a
   * stale lookup on the old suffix answers `unknown` from the very next
   * frame, not after some grace window — and the same connId keeps hosting,
   * under the new suffix, with a fresh secret (the old one dies with the old
   * suffix, same as everything else about it).
   *
   * The registry has no notion of "a mission is running" — GamePhase never
   * reaches this module — so the operator-facing "only between missions"
   * rule (AC2) is enforced by the CALLER before this frame is ever sent (see
   * server.html's `codesRotatable`), not here. What this function enforces is
   * the one fact the registry actually owns: only a record's own LIVE host
   * may rotate it — the same guard `hostAdmission` applies, and for the same
   * reason `admitHost`'s duplicate-`hello` note gives on the fleet side: a
   * grace-held record has no live connId that could ever satisfy it.
   */
  function hostRotate(connId) {
    const record = recordFor(connId);
    if (!record || record.host !== connId) return [fail(connId, 'rotate', 'not-hosting')];
    const minted = mintSuffix(
      data,
      (s) => records.has(recordKey(record.project, record.version, s)),
      randomInt,
    );
    if (!minted.ok) return [fail(connId, 'rotate', minted.reason)];

    // The peers of the OLD code are told it is gone — the same 'host-gone'
    // story a genuine drop tells them. The rotating host's OWN
    // `error`/`unreachable` notification (dropRecord's ordinary "your record
    // is gone" telling) is filtered out: this host asked for this, and
    // reporting its own rotation as a service fault would trip whatever else
    // that host's `onError` reacts to.
    const dropped = dropRecord(record, 'host-gone').filter((f) => f.to !== connId);

    const key = recordKey(record.project, record.version, minted.suffix);
    const fresh = {
      key,
      project: record.project,
      version: record.version,
      suffix: minted.suffix,
      namespace: record.namespace,
      host: connId,
      admission: 'open',
      peers: new Set(),
      createdAt: now(),
      lastSeen: now(),
      secret: mintSecret(),
      graceUntil: null,
    };
    records.set(key, fresh);
    const conn = conns.get(connId);
    if (conn) conn.key = key;

    return [
      ...dropped,
      out(connId, {
        type: 'hosted',
        code: { ...codeOf(fresh), secret: fresh.secret },
        admission: fresh.admission,
      }),
    ];
  }

  function dropRecord(record, reason) {
    const frames = [];
    for (const peer of record.peers) {
      // A relayed peer's game link IS this record — unlike a DataChannel, which
      // outlives the record that introduced it — so it is told the relay is
      // gone as well as the record. `relay-closed` is the frame its transport
      // turns into an ordinary link failure, so it reconnects rather than
      // sitting on a socket that will never carry another game frame.
      if (relay.keyFor(peer) === record.key) {
        frames.push(out(peer, { type: 'relay-closed', reason }));
        relay.detach(peer);
      }
      frames.push(out(peer, { type: 'closed', reason }));
      const c = conns.get(peer);
      if (c) c.key = null;
    }
    relay.detach(record.host);
    record.peers.clear();
    records.delete(record.key);
    // Grace-held (issue #1115): no live host connId to tell — the host that
    // is going to hear about this is whichever one reclaims (or fails to)
    // next, not a connection that already left.
    if (record.host) {
      const hostConn = conns.get(record.host);
      if (hostConn) hostConn.key = null;
      // Tell the host too, not just its joiners — an idle-past-TTL sweep drops
      // a record the host never asked to close, and until now the host's own
      // viewscreen kept showing a code the service had already forgotten.
      // 'error'/'unreachable' rather than 'closed': it is the frame
      // createRendezvousHost's host-side `handle()` already turns into "the
      // record is gone" teardown (gui/rendezvous-transport.js), the same
      // reason its own socket.onerror/onclose report, so a stale code clears
      // on its own instead of surviving the record that backed it.
      frames.push(out(record.host, { type: 'error', reason: 'unreachable' }));
    }
    return frames;
  }

  /**
   * This record's host socket died WITHOUT an explicit `host-close` — a
   * transient signalling loss (issue #1115): a dropped WS, a Durable Object
   * hiccup, a phone radio killing a backgrounded tab's connection. The record
   * SURVIVES — suffix, secret, admission state and presence untouched — held
   * for `reclaimGraceMs` with no live host to relay signalling to. A `join`
   * against it during that window answers the retryable `unreachable` (see
   * `clientJoin`) rather than the suffix going cold outright, and a joiner
   * already mid-reconnect keeps retrying through exactly that on its own
   * backoff. `hostOpen`'s `resume` handling is the only way back in before
   * the deadline; nobody presenting it in time means `expireStale()`
   * eventually drops this record for good, the same as any other whose host
   * never comes back.
   */
  function enterGrace(record) {
    record.host = null;
    record.graceUntil = now() + reclaimGraceMs;
    return [];
  }

  /**
   * A host socket dying holds its record in grace (issue #1115), but a RELAYED
   * game link cannot outlive that socket the way a direct DataChannel can (issue
   * #1113): the worker has no host socket left to forward relayed frames to. So
   * every peer relaying through this record is told the relay is gone
   * (`relay-closed`, which its transport turns into an ordinary link failure so
   * it reconnects and retries the — now `unreachable`, retryable — code) and
   * detached, along with the host's own relay mailbox. The record itself
   * survives; the direct-WebRTC peers are left untouched, their P2P channels
   * outliving the signalling socket, which is the whole point of grace. This
   * mirrors `dropRecord`'s relay teardown without dropping the record or
   * closing its direct peers.
   */
  function detachGraceRelays(record) {
    const frames = [];
    for (const peer of record.peers) {
      if (relay.keyFor(peer) === record.key) {
        frames.push(out(peer, { type: 'relay-closed', reason: 'host-gone' }));
        relay.detach(peer);
      }
    }
    relay.detach(record.host);
    return frames;
  }

  // ── Client requests ──────────────────────────────────────────────────────

  /**
   * The lookup primitive: "does this code name a joinable host, and is it
   * admitting?" — with no side effect and no presence. `join` answers the same
   * question and then attaches, so the shipped phone goes straight there; this
   * verb is what a launcher, a diagnostic or a "check the code before I
   * commit" step in #1114's fleet flow asks. Kept in v1 deliberately (the PRD's
   * test decisions name issue/resolve), and role-gated exactly like `join`, so
   * a host socket cannot use it to read the client namespace.
   */
  function clientResolve(connId, frame) {
    const conn = conns.get(connId);
    if (!conn || conn.role !== ROLE_CLIENT) return [fail(connId, 'resolve', 'forbidden-role')];
    const capped = chargeLookup(conn, connId, 'resolve');
    if (capped) return capped;
    const found = resolveRequest(frame.code, askedNamespace(frame));
    if (!found.ok) return [fail(connId, 'resolve', found.reason)];
    const record = found.record;
    return [
      out(connId, {
        type: 'resolved',
        namespace: record.namespace,
        admission: record.admission,
      }),
    ];
  }

  function clientJoin(connId, frame) {
    const conn = conns.get(connId);
    if (!conn || conn.role !== ROLE_CLIENT) return [fail(connId, 'join', 'forbidden-role')];
    const capped = chargeLookup(conn, connId, 'join');
    if (capped) return capped;
    const found = resolveRequest(frame.code, askedNamespace(frame));
    if (!found.ok) return [fail(connId, 'join', found.reason)];
    const record = found.record;
    // A record grace-held after its host's socket died (issue #1115) is still
    // a perfectly good record — its admission state says so — but there is
    // nobody to relay signalling to right now. `unreachable` is retryable
    // (gui/rendezvous-transport.js's `isRetryableReason`), so a joiner already
    // mid-reconnect just keeps retrying on its own backoff until either the
    // original host reclaims it or the grace window runs out for good.
    if (!record.host) return [fail(connId, 'join', 'unreachable')];
    if (record.admission !== 'open') return [fail(connId, 'join', 'admission-closed')];
    // A full crew list reads to the guest exactly like a closed one, and it is
    // the same sentence on their phone. The cap is not a party size — it is
    // the bound that stops one record holding unbounded presence.
    if (!record.peers.has(connId) && record.peers.size >= maxPeers) {
      return [fail(connId, 'join', 'admission-closed')];
    }

    const left = conn.key && conn.key !== record.key ? leave(connId) : [];
    conn.key = record.key;
    record.peers.add(connId);

    return [
      ...left,
      out(connId, {
        type: 'joined',
        peer: connId,
        admission: record.admission,
        // What this host can answer on. A joiner that reads no 'webrtc' here
        // skips the whole direct ladder instead of spending ninety seconds
        // discovering the same thing (issue #1113).
        transports: record.transports,
      }),
      // Presence only. The joiner's build identity travels in-band on the
      // DataChannel, to the host that actually decides on it — relaying a copy
      // through here was ignored by the host and only widened the surface.
      out(record.host, { type: 'peer-joined', peer: connId }),
    ];
  }

  function leave(connId) {
    const record = recordFor(connId);
    const conn = conns.get(connId);
    if (conn) conn.key = null;
    if (!record) {
      // A relay attachment with no record left is bookkeeping from a record
      // that has already gone (issue #1113); drop it rather than leaking a
      // mailbox.
      relay.detach(connId);
      return [];
    }
    // The host's socket merely dying — this is `disconnect()`'s only route
    // into `leave()`, never the explicit `host-close` frame (`hostClose`
    // above handles that one directly) — holds the record in grace rather
    // than dropping it for good. See `enterGrace` and the module doc's "code
    // lifecycle" section (issue #1115). A relayed game link (issue #1113)
    // cannot outlive that dead host socket the way a direct DataChannel can,
    // so `detachGraceRelays` first tells this record's relay peers the relay
    // is gone and detaches them; the record itself is what survives for
    // reclaim.
    if (record.host === connId) return [...detachGraceRelays(record), ...enterGrace(record)];
    const relayed = detachRelay(connId, 'peer-left');
    if (!record.peers.delete(connId)) return relayed;
    return [...relayed, out(record.host, { type: 'peer-left', peer: connId })];
  }

  // ── Signalling relay ─────────────────────────────────────────────────────

  function relaySignal(connId, frame) {
    const conn = conns.get(connId);
    const record = recordFor(connId);
    if (!conn || !record) return [fail(connId, 'signal', 'not-joined')];
    const isHost = record.host === connId;
    const target = isHost ? frame.to : record.host;
    if (!target) return [fail(connId, 'signal', 'no-peer')];
    if (isHost && !record.peers.has(target)) return [fail(connId, 'signal', 'no-peer')];
    return [out(target, { type: 'signal', from: connId, payload: frame.payload })];
  }

  // ── The WebSocket game relay (issue #1113) ───────────────────────────────
  //
  // Everything below moves OPAQUE payloads between an already-joined client and
  // its record's host, over the sockets they are already holding. The bounds,
  // the class split and the reason a reliable overflow is fatal while a
  // snapshot overflow is not all live in src/relay.js; these functions are the
  // registry's half — who is allowed to ask, and who the frame goes to.

  /** Push a peer's mailboxes onto the wire, plus whatever the drain implies. */
  function flushRelay(owner) {
    const frames = relay.drain(owner).map((f) => out(owner, f));
    // A reliable queue that filled is a session that can no longer keep the
    // promise its delivery class makes. Say so and end it, rather than becoming
    // a quietly lossy reliable channel.
    //
    // Which session, precisely: the mailbox that overflowed belongs to ONE
    // (owner, sender) pair, and exactly one end of any such pair is a joiner —
    // the other is the record's host. It is the joiner's relay that ends, so a
    // burst from one phone costs that phone its session and leaves the rest of
    // the crew on the wire. Ending the HOST's attachment instead, which is what
    // a per-connection mailbox forced, left every other relayed crew member
    // answering `not-relaying` for the rest of the mission.
    const key = relay.keyFor(owner);
    const record = key ? records.get(key) : null;
    for (const source of relay.overflowedSources(owner)) {
      const joiner = record && record.host === owner ? source : owner;
      frames.push(out(joiner, { type: 'relay-closed', reason: 'relay-overflow' }));
      frames.push(...detachRelay(joiner, 'relay-overflow'));
    }
    return frames;
  }

  /**
   * Detach one relay peer and tell its host. Safe to call for a connection that
   * was never relaying, so every teardown path can call it unconditionally.
   */
  function detachRelay(peer, reason) {
    const key = relay.keyFor(peer);
    if (!key) return [];
    relay.detach(peer);
    const record = records.get(key);
    if (!record || record.host === peer) return [];
    return [out(record.host, { type: 'relay-peer-left', peer, reason: reason || 'closed' })];
  }

  /**
   * Attach a joined client to its record's relay.
   *
   * Deliberately gated on having JOINED first: the relay is the fallback for a
   * direct link that could not be built, not a way to reach a host without ever
   * resolving its code. Everything the join path decided — the typed lookup,
   * the admission state, the peer cap — therefore already applies, and this
   * verb adds only the relay's own bound on top.
   */
  function relayOpen(connId) {
    const conn = conns.get(connId);
    const record = recordFor(connId);
    if (!conn || conn.role !== ROLE_CLIENT) return [fail(connId, 'relay-open', 'forbidden-role')];
    if (!record || !record.peers.has(connId)) return [fail(connId, 'relay-open', 'not-joined')];
    if (record.admission !== 'open') return [fail(connId, 'relay-open', 'admission-closed')];

    const attached = relay.attach(connId, record.key);
    if (!attached.ok) return [fail(connId, 'relay-open', attached.reason)];
    // The host takes a mailbox of its own the first time anyone relays to it:
    // many phones to one host is the direction a burst actually arrives from,
    // and it needs the same bound. Uncounted, so it never costs a crew slot.
    relay.attach(record.host, record.key, { counted: false });

    // The HOST is told first, deliberately. The joiner's very next act is to
    // put its compatibility handshake on the relay, and a host that had not yet
    // built its side of the pair would drop it — invisible over a real socket,
    // where the two frames are separate messages in order, but immediate in any
    // adapter that dispatches a batch synchronously (the contract tests, and
    // tests/smoke/rendezvous-shim.js).
    return [
      out(record.host, { type: 'relay-peer', peer: connId, limits: relayAdvice() }),
      out(connId, {
        type: 'relay-ready',
        peer: connId,
        limits: relayAdvice(),
      }),
    ];
  }

  /**
   * The bounds both ends of a relayed link are told about, in the wire's own
   * snake_case. Sent to the joiner on `relay-ready` and to the host on
   * `relay-peer` from ONE place, because a host that believed a different
   * ceiling from its crew member would refuse frames the service would have
   * carried, or send frames it would not.
   */
  function relayAdvice() {
    return {
      max_frame_bytes: relay.limits.maxFrameBytes,
      max_queue_reliable: relay.limits.maxReliable,
      max_queue_snapshot: relay.limits.maxSnapshot,
      max_send_buffer_bytes: relay.limits.maxSendBufferBytes,
    };
  }

  /**
   * Carry one game frame. A client's frames go to its host; a host's go to the
   * `to` peer it names, and only to one that is actually relaying — a host may
   * not use this verb to reach a peer that has a perfectly good DataChannel.
   */
  function relayFrame(connId, frame) {
    const conn = conns.get(connId);
    const record = recordFor(connId);
    if (!conn || !record) return [fail(connId, 'relay', 'not-joined')];
    const isHost = record.host === connId;
    if (!isHost && relay.keyFor(connId) !== record.key) {
      return [fail(connId, 'relay', 'not-relaying')];
    }
    const target = isHost ? frame.to : record.host;
    if (!target) return [fail(connId, 'relay', 'no-peer')];
    if (isHost && relay.keyFor(target) !== record.key) {
      return [fail(connId, 'relay', 'no-peer')];
    }
    if (!isRelayClass(frame.class)) return [fail(connId, 'relay', 'malformed')];

    const queued = relay.enqueue(target, connId, frame.class, {
      type: 'relay',
      from: connId,
      class: frame.class,
      payload: frame.payload,
    }, frame.payload);
    if (!queued.ok) return [fail(connId, 'relay', queued.reason)];

    const frames = flushRelay(target);
    // Tell the SENDER what its own traffic cost, not the receiver: the sender
    // is the one that can slow down, and on the host side it is the one whose
    // operator is looking at a diagnostics readout.
    //
    // The number is the pair's RUNNING TOTAL rather than this enqueue's delta.
    // A delta is 1 essentially always — a queue at its bound sheds one frame
    // per arrival — so a readout folding deltas showed "1" for the whole
    // mission however many hundreds were actually lost.
    if (queued.dropped > 0) {
      frames.push(out(connId, {
        type: 'relay-degraded',
        peer: target,
        class: frame.class,
        dropped: queued.totalDropped,
      }));
    }
    return frames;
  }

  /**
   * Stepping off the relay.
   *
   * Without `to` this is a CLIENT saying it no longer needs carrying (it got a
   * direct link, or it is leaving). With `to` it is a HOST evicting one crew
   * member — the reserved-token refusal and the duplicate-token dance in
   * server.html, which for a WebRTC peer sever real DataChannels the phone
   * observes, and for a relayed peer previously severed nothing but local
   * JavaScript: the evicted device kept its channels "open", kept sending
   * commands the host dropped on the floor, and had no diagnosis available at
   * either end.
   *
   * A host may NOT send the un-addressed form: it would detach its own
   * attachment and silently end the relay for every crew member on the record.
   */
  function relayClose(connId, frame) {
    const record = recordFor(connId);
    const target = frame && frame.to;
    const isHost = !!record && record.host === connId;
    if (!target) {
      if (isHost) return [fail(connId, 'relay-close', 'forbidden-role')];
      return detachRelay(connId, 'closed');
    }
    if (!isHost) return [fail(connId, 'relay-close', 'forbidden-role')];
    if (relay.keyFor(target) !== record.key) return [fail(connId, 'relay-close', 'no-peer')];
    // The joiner already treats `relay-closed` as the end of its game path, so
    // it fails and re-enters its reconnect loop exactly as a severed
    // DataChannel peer does.
    return [
      out(target, { type: 'relay-closed', reason: 'host-closed' }),
      ...detachRelay(target, 'host-closed'),
    ];
  }

  // ── Public surface ───────────────────────────────────────────────────────

  return {
    protocol: RENDEZVOUS_PROTOCOL,

    /**
     * The adapter reporting whether it can currently put bytes on `connId`'s
     * socket — the only backpressure signal a Durable Object has, since
     * Cloudflare's WebSocket exposes no `bufferedAmount` (see src/relay.js).
     *
     * While a relay peer is unwritable its mailbox holds, shedding snapshot
     * frames at the authored depth and never shedding reliable ones. Reporting
     * it writable again drains whatever survived, which is why this returns
     * frames rather than nothing.
     */
    setWritable(connId, writable) {
      relay.setWritable(connId, writable);
      return writable ? flushRelay(connId) : [];
    },

    /** Register a socket. `role` comes from the endpoint the adapter served. */
    connect(connId, role) {
      conns.set(connId, {
        role: role === ROLE_HOST ? ROLE_HOST : ROLE_CLIENT,
        key: null,
        lookups: 0,
      });
      return [out(connId, { type: 'ready', role, protocol: RENDEZVOUS_PROTOCOL })];
    },

    /** Feed one decoded frame in; get the frames to send out. */
    receive(connId, frame) {
      if (!conns.has(connId)) return [fail(connId, 'unknown', 'not-connected')];
      // Every inbound frame from a hosting connection refreshes ITS record's
      // `lastSeen` — before the sweep below runs, so a live host proves
      // itself with this very frame rather than needing a second one to
      // survive the same call. hostOpen stamps the initial value; this is
      // what keeps it fresh for as long as the host keeps talking.
      const conn = conns.get(connId);
      if (conn.role === ROLE_HOST && conn.key) {
        const hostRecord = records.get(conn.key);
        if (hostRecord) hostRecord.lastSeen = now();
      }
      // Sweep first, so a request never resolves a record whose TTL has run
      // out, and the joiners of an expired one are told before anything else
      // this frame produces.
      const expired = expireStale();
      const handled = (() => {
        if (!frame || typeof frame !== 'object') return [fail(connId, 'unknown', 'malformed')];
        if (frame.v !== RENDEZVOUS_PROTOCOL) {
          return [fail(connId, frame.type || 'unknown', 'unsupported-protocol')];
        }
        switch (frame.type) {
          case 'host-open': return hostOpen(connId, frame);
          case 'host-admission': return hostAdmission(connId, frame);
          case 'rotate': return hostRotate(connId);
          case 'host-close': return hostClose(connId);
          case 'resolve': return clientResolve(connId, frame);
          case 'join': return clientJoin(connId, frame);
          case 'signal': return relaySignal(connId, frame);
          case 'relay-open': return relayOpen(connId);
          case 'relay': return relayFrame(connId, frame);
          case 'relay-close': return relayClose(connId, frame);
          case 'leave': return leave(connId);
          default: return [fail(connId, String(frame.type || 'unknown'), 'malformed')];
        }
      })();
      return [...expired, ...handled];
    },

    /** Socket closed. Drops presence and, for a host, its whole record. */
    disconnect(connId) {
      const frames = leave(connId);
      conns.delete(connId);
      return frames;
    },

    /**
     * Read-only view for diagnostics and tests: what is registered, in which
     * namespace and release, admitting or not, with how many peers.
     *
     * It carries NO SUFFIX, no peer id and no connection id. The suffix is the
     * private client code — the one secret this whole feature exists to
     * protect — and a shape that includes it is a diagnostics endpoint one
     * route away from handing every live code out. Count records here; get a
     * code from the `hosted` frame that issued it.
     */
    snapshot() {
      return [...records.values()].map((r) => ({
        namespace: r.namespace,
        version: r.version,
        admission: r.admission,
        peers: r.peers.size,
        // How many of those peers gave up on a direct link and are being
        // carried by the service itself. A count, like `peers` — no id, for the
        // same reason the suffix is absent.
        relayPeers: relay.peersFor(r.key).length,
      }));
    },
  };
}
