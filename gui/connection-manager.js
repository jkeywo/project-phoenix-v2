import { hasRelayServer } from '../packages/transport/src/connection-manager.js';
import { nextBackoffDelay, connectTimeoutMs, probeTurnRelay, candidateType, readSelectedPair } from '../packages/transport/src/connection-manager.js';
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

export * from '../packages/transport/src/connection-manager.js';
if (typeof window !== 'undefined') {
  window.fetchIceServers = fetchIceServers;
  window.probeTurnRelay = probeTurnRelay;
}
