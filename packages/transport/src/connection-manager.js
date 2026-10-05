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

export function hasRelayServer(servers) {
  return (servers || []).some(s => {
    const urls = Array.isArray(s.urls) ? s.urls : [s.urls];
    return urls.some(u => typeof u === 'string' && (u.startsWith('turn:') || u.startsWith('turns:')));
  });
}
