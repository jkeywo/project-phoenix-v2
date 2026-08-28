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
 *   signal                   peer-left                   signal
 *   host-close               signal                      closed
 *                            error                       error
 * client → service
 *   resolve | join | signal | leave
 *
 * `resolve` and `join` carry an optional `namespace` naming which typed field
 * the code was entered into — the crew field on a phone, the fleet field on a
 * ship host (issue #1114). Absent means `client`, so a frame written before
 * that issue means exactly what it always did. It is what keeps "you typed the
 * other kind of code" answerable now that BOTH namespaces are joinable: a fleet
 * code is a perfectly good record, just not one a phone may attach to.
 *
 * Failures are always an `error` frame naming the request and one stable
 * machine `reason`; the phone maps that reason to a strings.csv id through
 * gui/join-code.js's `reasonStringId`, so the service never ships prose.
 *
 * ── What v1 deliberately does NOT do ────────────────────────────────────────
 *
 * All state here is IN MEMORY, in one Durable Object instance: records die
 * with the instance, and nothing is written to storage. That is the whole of
 * #1111's scope. A code therefore cannot outlive its host socket, survive a
 * worker redeploy, or be rebound to a replacement host — code rotation and
 * stable-code-across-a-drop are #1115 and #1120, and they are what would add
 * persistence. The `record_ttl_seconds` sweep below is not persistence: it is
 * an idle timeout measured from `lastSeen`, refreshed by every inbound frame
 * from the record's own host, not from when the record was created — so a
 * host that has been live for days is never swept merely for being old. It
 * catches the case a `close` event never arrived for (an evicted instance, a
 * half-open socket): the host has gone quiet, and once nothing has been heard
 * from it for that long, the code cannot be held forever.
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

function defaultRandomInt(n) {
  // crypto is present in Workers, in browsers and in Node 20+.
  const buf = new Uint32Array(1);
  crypto.getRandomValues(buf);
  return buf[0] % n;
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
    let frames = [];
    for (const record of [...records.values()]) {
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
    };
    records.set(key, record);
    conn.key = key;

    return [
      out(connId, {
        type: 'hosted',
        code: {
          full: composeJoinCode({ project, version, suffix: minted.suffix }),
          suffix: minted.suffix,
          project,
          version,
          namespace,
        },
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
    return dropRecord(record, 'host-gone');
  }

  function dropRecord(record, reason) {
    const frames = [];
    for (const peer of record.peers) {
      frames.push(out(peer, { type: 'closed', reason }));
      const c = conns.get(peer);
      if (c) c.key = null;
    }
    record.peers.clear();
    records.delete(record.key);
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
    if (!record) return [];
    if (record.host === connId) return dropRecord(record, 'host-gone');
    if (!record.peers.delete(connId)) return [];
    return [out(record.host, { type: 'peer-left', peer: connId })];
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

  // ── Public surface ───────────────────────────────────────────────────────

  return {
    protocol: RENDEZVOUS_PROTOCOL,

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
          case 'host-close': return hostClose(connId);
          case 'resolve': return clientResolve(connId, frame);
          case 'join': return clientJoin(connId, frame);
          case 'signal': return relaySignal(connId, frame);
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
      }));
    },
  };
}
