/**
 * gui/host-peer-routing.js — the pure routing decision behind server.html's
 * `routeOutbound()` (issue #1230).
 *
 * Every `OutboundMessage` Rust flushes reaches `routeOutbound(target, payload,
 * deliveryClass)` with a `target` string (`'all'`, `'token:<id>'`, or
 * `'except:<id>'`) and a `deliveryClass` (`'reliable'` — the default token
 * connection — or `'snapshot'` — the unreliable/unordered channel opened
 * alongside it for `AiState` pushes). `outboundTargets()` is the target ⇒
 * connection-list resolution ONLY: which PeerJS `DataConnection`s the payload
 * goes out on. server.html keeps the actual `.send(payload)` calls and the
 * live `tokenConns`/`tokenSnapshotConns` Maps (`new Map()`, populated as
 * phones identify and drop as they disconnect) — this module never touches
 * either Map, so it stays reachable from vitest with two plain
 * `Map([[token, fakeConn], ...])` fixtures instead of a real PeerJS session.
 *
 * ## The snapshot ⇒ reliable fallback
 *
 * A `snapshot` delivery prefers `tokenSnapshotConns` (the unreliable channel)
 * but falls back to `tokenConns` (the reliable one) for any token that opened
 * a peer connection before its snapshot channel finished negotiating — the
 * gap is per-token, not global, so `'all'`/`'except:'` fall back independently
 * per missing token rather than downgrading the whole broadcast. A `reliable`
 * delivery has no fallback: `tokenConns` is the only channel it will ever use.
 *
 * `'token:<id>'` returns AT MOST one connection — the same first-match-wins
 * shape `routeOutbound` always had (primary if open, else the fallback, never
 * both) — while `'all'` and `'except:<id>'` can return several. A token whose
 * connection exists but is not open (`c.open` — PeerJS's own flag — nor its
 * `readyState === 'open'`, the WebRTC data channel's own state, covering both
 * connection implementations this codebase has carried) is skipped exactly
 * like the original inline loop skipped it, on every branch.
 */

/** True when a PeerJS DataConnection (or a raw RTCDataChannel-shaped stub) is
 *  ready to send. */
function isOpen(conn) {
  return !!(conn && (conn.open || conn.readyState === 'open'));
}

/**
 * Resolve which connections a `routeOutbound` call should send on.
 *
 * @param {string} target 'all' | 'token:<id>' | 'except:<id>'
 * @param {string} deliveryClass 'reliable' | 'snapshot' (anything else behaves
 *   like 'reliable' — routeOutbound's own `useSnapshot` check was `===
 *   'snapshot'`, never an allowlist of the reliable spelling).
 * @param {{tokenConns: Map<string, object>, tokenSnapshotConns: Map<string, object>}} conns
 *   the host page's live per-token connection maps (unmutated).
 * @returns {object[]} open connections to `.send()` the payload on, in
 *   Map-iteration order; empty when nothing matches or nothing is open.
 */
export function outboundTargets(target, deliveryClass, { tokenConns, tokenSnapshotConns }) {
  const useSnapshot = deliveryClass === 'snapshot';
  const primary = useSnapshot ? tokenSnapshotConns : tokenConns;
  const fallback = useSnapshot ? tokenConns : null;

  if (target === 'all') {
    const sent = new Set();
    const out = [];
    for (const [t, c] of primary) {
      if (isOpen(c)) { out.push(c); sent.add(t); }
    }
    if (fallback) {
      for (const [t, c] of fallback) {
        if (!sent.has(t) && isOpen(c)) out.push(c);
      }
    }
    return out;
  }

  if (target.startsWith('token:')) {
    const t = target.slice(6);
    const c = primary.get(t);
    if (isOpen(c)) return [c];
    if (fallback) {
      const fc = fallback.get(t);
      if (isOpen(fc)) return [fc];
    }
    return [];
  }

  if (target.startsWith('except:')) {
    const exc = target.slice(7);
    const sent = new Set();
    const out = [];
    for (const [t, c] of primary) {
      if (t !== exc && isOpen(c)) { out.push(c); sent.add(t); }
    }
    if (fallback) {
      for (const [t, c] of fallback) {
        if (!sent.has(t) && t !== exc && isOpen(c)) out.push(c);
      }
    }
    return out;
  }

  // Unrecognised target shape: routeOutbound's original `if/else if` chain
  // fell through and sent nothing — preserved rather than throwing, since a
  // future OutboundMessage target this module doesn't know about yet should
  // degrade to "delivered nowhere", not a page crash.
  return [];
}

// Expose for the classic-script consumer (server.html is not a module).
if (typeof window !== 'undefined') {
  window.hostPeerRouting = { outboundTargets };
}
