/**
 * gui/connection-manager.js — ICE/TURN configuration and connection timing
 * policy for both pages.
 *
 * Everything here is pure and transport-shaped rather than transport-specific:
 * which STUN/TURN servers to offer, whether a relay is actually allocatable
 * from this network, how long to wait for a connect attempt, and how long to
 * wait before the next one. The Phoenix transport
 * (gui/rendezvous-transport.js) is the only consumer of the timing half; both
 * pages consume the ICE half.
 *
 * It used to also own `class ConnectionManager`, the PeerJS client lifecycle.
 * Issue #1112 retired PeerJS entirely, and the reconnect/backoff/snapshot
 * behaviour that class carried now lives in the joiner half of
 * gui/rendezvous-transport.js — one connection lifecycle, not two. The module
 * keeps its name because these exports never were PeerJS's: `fetchIceServers`
 * is the single source of truth for the credential-worker → OpenRelay fallback
 * policy, and `nextBackoffDelay`/`connectTimeoutMs` are the two schedules the
 * transport is tuned by.
 */

export function defaultIceServers() {
  // STUN only. TURN relay comes from the credential worker via
  // fetchIceServers(), with openRelayFallbackServers() as the last resort —
  // never bake TURN entries into this base list, or relaySource would lie.
  return [
    { urls: 'stun:stun.l.google.com:19302' },
    { urls: 'stun:stun1.l.google.com:19302' },
  ];
}

export function openRelayFallbackServers() {
  // Metered's free shared OpenRelay TURN, used only when the credential
  // worker is unreachable. Note the staticauth. hostname: the bare
  // openrelay.metered.ca the code used pre-2026-08 has no DNS records any
  // more (NODATA from public resolvers), while staticauth. is what Metered's
  // docs currently advertise. Shared and rate-limited — better than no relay
  // on CGNAT, but callers should surface that they're on the fallback.
  return [
    { urls: 'turn:staticauth.openrelay.metered.ca:80',  username: 'openrelayproject', credential: 'openrelayproject' },
    { urls: 'turn:staticauth.openrelay.metered.ca:443', username: 'openrelayproject', credential: 'openrelayproject' },
    { urls: 'turns:staticauth.openrelay.metered.ca:443?transport=tcp', username: 'openrelayproject', credential: 'openrelayproject' },
  ];
}

/** True if any entry in an iceServers list is a TURN/TURNS relay. */
export function hasRelayServer(servers) {
  return (servers || []).some(s => {
    const urls = Array.isArray(s.urls) ? s.urls : [s.urls];
    return urls.some(u => typeof u === 'string' && (u.startsWith('turn:') || u.startsWith('turns:')));
  });
}

/**
 * Fetch relay credentials from the worker and combine with the STUN base.
 * Returns { servers, relayAvailable, relaySource }:
 *   relaySource 'worker'    — dedicated Metered.ca credentials, the good path
 *   relaySource 'openrelay' — worker unreachable; free shared OpenRelay TURN
 *                             appended instead (congested, rate-limited —
 *                             callers should show a mild degraded notice)
 *   relaySource null        — no TURN relay at all (worker responded ok but
 *                             without TURN entries); on CGNAT/hotspot networks
 *                             the connection will almost certainly fail, so
 *                             callers must warn rather than retry silently.
 */
export async function fetchIceServers() {
  const base = defaultIceServers();
  try {
    const r = await fetch('https://phoenix-turn-credentials.project-phoenix.workers.dev');
    if (r.ok) {
      const extra = await r.json();
      console.log(`[ICE] Metered.ca returned ${extra.length} server(s) — appending to base list`);
      const servers = [...base, ...extra];
      const relayAvailable = hasRelayServer(servers);
      return { servers, relayAvailable, relaySource: relayAvailable ? 'worker' : null };
    }
    console.warn('[ICE] Metered.ca fetch returned', r.status, '— falling back to shared OpenRelay TURN');
  } catch (e) {
    console.warn('[ICE] Metered.ca fetch failed — falling back to shared OpenRelay TURN:', e.message);
  }
  return {
    servers: [...base, ...openRelayFallbackServers()],
    relayAvailable: true,
    relaySource: 'openrelay',
  };
}

/**
 * Backoff delay schedule for reconnect attempts: doubles each attempt starting
 * from `initialMs`, capped at `maxMs`. `attempt` is 0-indexed (0 = first retry).
 * Pure function so it's unit-testable without touching timers or a transport.
 *
 * @param {number} attempt 0-indexed retry attempt number
 * @param {number} [initialMs] delay for the first attempt (default 100ms)
 * @param {number} [maxMs] cap on the delay (default 30_000ms)
 * @returns {number} delay in milliseconds before the next attempt
 */
export function nextBackoffDelay(attempt, initialMs = 100, maxMs = 30_000) {
  const raw = initialMs * Math.pow(2, Math.max(0, attempt));
  return Math.min(raw, maxMs);
}

/**
 * Per-attempt DataChannel connect timeout. TURN allocation over TCP/TLS on
 * cellular can legitimately take longer than the old flat 8s, so later
 * attempts wait longer before giving up: 8s, then 16s, then 30s thereafter.
 * Pure function, unit-testable like nextBackoffDelay.
 *
 * @param {number} attempt 0-indexed connect attempt number
 * @returns {number} timeout in milliseconds for this attempt
 */
export function connectTimeoutMs(attempt, schedule = [8_000, 16_000, 30_000]) {
  const i = Math.min(Math.max(0, attempt), schedule.length - 1);
  return schedule[i];
}

/**
 * Probe whether a TURN relay is actually allocatable from this network.
 * Opens a throwaway RTCPeerConnection with iceTransportPolicy 'relay' — under
 * that policy any candidate that surfaces IS a relay candidate, so the first
 * one proves reachability. Resolves 'reachable', 'unreachable' (TURN in the
 * list but no relay candidate within timeoutMs), or 'unavailable' (no TURN
 * entries / no WebRTC).
 */
export function probeTurnRelay(iceServers, timeoutMs = 5_000) {
  return new Promise(resolve => {
    if (typeof RTCPeerConnection === 'undefined' || !hasRelayServer(iceServers)) {
      resolve('unavailable');
      return;
    }
    let pc;
    try {
      pc = new RTCPeerConnection({ iceServers, iceTransportPolicy: 'relay' });
    } catch (_) {
      resolve('unavailable');
      return;
    }
    let settled = false;
    const done = verdict => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try { pc.close(); } catch (_) { /* already closed */ }
      resolve(verdict);
    };
    const timer = setTimeout(() => done('unreachable'), timeoutMs);
    pc.onicecandidate = e => {
      if (e.candidate && e.candidate.candidate) done('reachable');
    };
    pc.createDataChannel('turn-probe');
    pc.createOffer()
      .then(offer => pc.setLocalDescription(offer))
      .catch(() => done('unavailable'));
  });
}

/**
 * Candidate type ('host' | 'srflx' | 'relay' | 'prflx') from an
 * RTCIceCandidate, falling back to parsing the SDP string on browsers that
 * don't populate `.type`.
 */
export function candidateType(candidate) {
  if (!candidate) return null;
  if (candidate.type) return candidate.type;
  const m = /\styp\s+(\S+)/.exec(candidate.candidate || '');
  return m ? m[1] : null;
}

/**
 * Read the candidate pair ICE actually chose out of an `RTCPeerConnection`.
 *
 * This is the one question the existing readout could not answer and the field
 * kit most needs: "candidates: host, srflx, relay" says what was OFFERED, not
 * what carried the traffic — and on a hotspot the difference between a
 * server-reflexive pair and a relayed one is the difference between a working
 * network and a working TURN worker. `url` on the local candidate names WHICH
 * relay, which is how a tester tells the dedicated credential worker from the
 * free shared fallback without trusting a config field.
 *
 * Async, tolerant of everything: `getStats` is absent in the smoke suite's fake
 * peer connection and may reject on a connection that has already closed, and a
 * diagnostics line is never worth a thrown error.
 *
 * @returns {Promise<{local:string,remote:string,protocol:string,url:string}|null>}
 */
export async function readSelectedPair(pc) {
  if (!pc || typeof pc.getStats !== 'function') return null;
  let stats;
  try {
    stats = await pc.getStats();
  } catch {
    return null;
  }
  if (!stats || typeof stats.forEach !== 'function') return null;

  const byId = new Map();
  let pair = null;
  stats.forEach((report) => {
    if (!report || !report.id) return;
    byId.set(report.id, report);
    if (report.type !== 'candidate-pair') return;
    // `selected` is Firefox's spelling; `nominated` + state 'succeeded' is
    // everyone else's. Take whichever the browser offers rather than insisting
    // on one and reporting nothing on the other.
    if (report.selected || (report.nominated && report.state === 'succeeded')) pair = report;
  });
  if (!pair) return null;

  const local = byId.get(pair.localCandidateId) || {};
  const remote = byId.get(pair.remoteCandidateId) || {};
  return {
    local: local.candidateType || '',
    remote: remote.candidateType || '',
    protocol: local.relayProtocol || local.protocol || '',
    url: local.url || '',
  };
}

// Published for the classic-script halves of both pages, which cannot import.
// There is no `window.connectionManager` any more: the live link a page speaks
// through is the Phoenix joiner, and client.html publishes that one as
// `window.phoenixLink` (issue #1112).
if (typeof window !== 'undefined') {
  window.fetchIceServers = fetchIceServers;
  window.probeTurnRelay = probeTurnRelay;
}
