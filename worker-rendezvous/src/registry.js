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
 * Failures are always an `error` frame naming the request and one stable
 * machine `reason`; the phone maps that reason to a strings.csv id through
 * gui/join-code.js's `reasonStringId`, so the service never ships prose.
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
} from '../../gui/join-code.js';

/** Frame-vocabulary revision. Bump only for an incompatible change. */
export const RENDEZVOUS_PROTOCOL = 1;

/** Roles a socket may open with. The adapter derives these from the path. */
export const ROLE_HOST = 'host';
export const ROLE_CLIENT = 'client';

const recordKey = (project, version, suffix) =>
  `${String(project).toLowerCase()}|${String(version).toLowerCase()}|${suffix}`;

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
 *   #1111 ships `['client']`: the server namespace must EXIST so a wrong-type
 *   answer is possible, but joining it is #1114's to enable — by adding one
 *   entry here, not by reshaping the lookup.
 */
export function createRegistry({
  data,
  randomInt = defaultRandomInt,
  now = () => Date.now(),
  joinableNamespaces = [NAMESPACE_CLIENT],
} = {}) {
  if (!data) throw new Error('rendezvous: no join-code format data');

  /** @type {Map<string, object>} recordKey → host record */
  const records = new Map();
  /** @type {Map<string, object>} connection id → {role, key} */
  const conns = new Map();

  const joinable = new Set(joinableNamespaces);

  const out = (to, frame) => ({ to, frame: { v: RENDEZVOUS_PROTOCOL, ...frame } });
  const fail = (to, request, reason, detail) =>
    out(to, { type: 'error', request, reason, ...(detail ? { detail } : {}) });

  const recordFor = (connId) => {
    const c = conns.get(connId);
    return c && c.key ? records.get(c.key) || null : null;
  };

  /**
   * Typed lookup. Returns `{ok:true, record}` or `{ok:false, reason}` with one
   * of the three failures the PRD demands kept apart: `unknown`, `wrong-type`
   * and `version-mismatch`. Nothing beyond the class of failure is revealed.
   */
  function lookup(project, version, suffix) {
    const exact = records.get(recordKey(project, version, suffix));
    if (exact) {
      const ns = namespaceOf(exact.project, data);
      if (!joinable.has(ns)) return { ok: false, reason: 'wrong-type' };
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

  function resolveRequest(code) {
    const parsed = parseJoinCode(code, NAMESPACE_CLIENT, data);
    if (!parsed.ok) return { ok: false, reason: parsed.reason };
    return lookup(parsed.project, parsed.version, parsed.suffix);
  }

  // ── Host requests ────────────────────────────────────────────────────────

  function hostOpen(connId, frame) {
    const conn = conns.get(connId);
    if (!conn || conn.role !== ROLE_HOST) return [fail(connId, 'host-open', 'forbidden-role')];
    if (conn.key) return [fail(connId, 'host-open', 'already-hosting')];

    const namespace = frame.namespace || NAMESPACE_CLIENT;
    const project = projectGuidFor(namespace, data);
    if (!project) return [fail(connId, 'host-open', 'wrong-type')];
    const version = frame.version || versionGuid(data);

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
      // The host's delivery stamp, relayed to joiners as ADVISORY discovery
      // help. The authoritative protocol/content check is the host's own,
      // over the DataChannel — see delivery::check_join_stamp.
      stamp: typeof frame.stamp === 'string' ? frame.stamp : null,
      admission: 'open',
      peers: new Set(),
      createdAt: now(),
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
    return frames;
  }

  // ── Client requests ──────────────────────────────────────────────────────

  function clientResolve(connId, frame) {
    const found = resolveRequest(frame.code);
    if (!found.ok) return [fail(connId, 'resolve', found.reason)];
    const record = found.record;
    return [
      out(connId, {
        type: 'resolved',
        namespace: record.namespace,
        admission: record.admission,
        host_stamp: record.stamp,
      }),
    ];
  }

  function clientJoin(connId, frame) {
    const conn = conns.get(connId);
    if (!conn || conn.role !== ROLE_CLIENT) return [fail(connId, 'join', 'forbidden-role')];
    const found = resolveRequest(frame.code);
    if (!found.ok) return [fail(connId, 'join', found.reason)];
    const record = found.record;
    if (record.admission !== 'open') return [fail(connId, 'join', 'admission-closed')];

    const left = conn.key && conn.key !== record.key ? leave(connId) : [];
    conn.key = record.key;
    record.peers.add(connId);

    return [
      ...left,
      out(connId, {
        type: 'joined',
        peer: connId,
        admission: record.admission,
        host_stamp: record.stamp,
      }),
      out(record.host, {
        type: 'peer-joined',
        peer: connId,
        // The joiner's declared build identity, relayed verbatim. The host
        // decides; the service never refuses a join on it.
        stamp: typeof frame.stamp === 'string' ? frame.stamp : null,
      }),
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
      conns.set(connId, { role: role === ROLE_HOST ? ROLE_HOST : ROLE_CLIENT, key: null });
      return [out(connId, { type: 'ready', role, protocol: RENDEZVOUS_PROTOCOL })];
    },

    /** Feed one decoded frame in; get the frames to send out. */
    receive(connId, frame) {
      if (!conns.has(connId)) return [fail(connId, 'unknown', 'not-connected')];
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
    },

    /** Socket closed. Drops presence and, for a host, its whole record. */
    disconnect(connId) {
      const frames = leave(connId);
      conns.delete(connId);
      return frames;
    },

    /**
     * Read-only view for diagnostics and tests: what is registered, in which
     * namespace, with how many peers. Never includes a host's stamp or a peer
     * id, so a diagnostics endpoint cannot leak a live session's metadata.
     */
    snapshot() {
      return [...records.values()].map((r) => ({
        namespace: r.namespace,
        version: r.version,
        suffix: r.suffix,
        admission: r.admission,
        peers: r.peers.size,
      }));
    },
  };
}
