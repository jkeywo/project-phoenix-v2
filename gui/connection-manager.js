// Side-effect import: strings-boot's top-level await blocks this module until
// the string table is loaded, so localiseTree below can never run against an
// empty table and leak raw ids into a console. Do not drop this in favour of
// relying on <script> ordering in client.html — that only holds if every entry
// point keeps strings-boot first.
import './strings-boot.js';
import { localiseTree } from './strings.js';

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
 * Pure function so it's unit-testable without touching timers/PeerJS.
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

export class ConnectionManager {
  constructor() {
    this.conn = null;
    this._snapshotConn = null;
    this._snapshotAvailable = false;
    this.peer = null;
    this._identSent = false;
    this._retryTimer = null;
    this._retryAttempt = 0;
    this._opts = null;
    this._hostPeerId = null;
    this._clientPeer = null;
    this._generation = 0;
  }

  get connected() {
    return this.conn !== null && this.conn.open;
  }

  connect(hostPeerId, { iceServers, onData, onStatus, onError, onLog, onDiag, getIdent } = {}) {
    const Peer = typeof window !== 'undefined' ? window.Peer : null;
    if (!Peer || !hostPeerId) return;

    // Stash for retryNow()/backoff-triggered reconnects, which re-run this
    // same setup against a fresh Peer.
    this._hostPeerId = hostPeerId;
    this._opts = { iceServers, onData, onStatus, onError, onLog, onDiag, getIdent };
    this._identSent = false;
    this._clearRetryTimer();
    const generation = ++this._generation;

    const clientPeer = new Peer({ config: { iceServers: iceServers || defaultIceServers() } });
    this.peer = clientPeer;
    this._clientPeer = clientPeer;

    if (onLog) onLog('[PeerJS] connecting to host peer ID: ' + hostPeerId);
    if (onDiag) onDiag({ event: 'signaling', state: 'connecting' });

    clientPeer.on('open', () => {
      if (!this._isCurrentPeer(clientPeer, generation)) return;
      if (onLog) onLog('[PeerJS] client peer open — starting DataChannel connect');
      if (onDiag) onDiag({ event: 'signaling', state: 'open' });
      let connectTimeout;

      const startConnect = () => {
        if (!this._isCurrentPeer(clientPeer, generation)) return;
        if (onLog) onLog(`[PeerJS] connect attempt ${this._retryAttempt + 1}—`);
        if (onStatus) onStatus('connecting');
        if (onDiag) onDiag({ event: 'attempt', attempt: this._retryAttempt + 1 });
        const conn = clientPeer.connect(hostPeerId, { reliable: true });
        this.conn = conn;

        // Candidate types gathered this attempt. "No relay" here on a failing
        // hotspot/CGNAT network is the smoking gun for a TURN problem.
        const gathered = new Set();
        const pc = conn.peerConnection;
        if (pc) {
          pc.addEventListener('iceconnectionstatechange', () => {
            if (onLog) onLog(`[ICE] state — ${pc.iceConnectionState}`);
            if (onDiag) onDiag({ event: 'ice-state', state: pc.iceConnectionState });
          });
          pc.addEventListener('icegatheringstatechange', () => {
            if (onLog) onLog(`[ICE] gathering — ${pc.iceGatheringState}`);
          });
          pc.addEventListener('icecandidate', e => {
            const type = candidateType(e.candidate);
            if (type && !gathered.has(type)) {
              gathered.add(type);
              if (onLog) onLog(`[ICE] gathered ${type} candidate`);
              if (onDiag) onDiag({ event: 'candidates', types: [...gathered] });
            }
          });
        }

        const timeoutMs = connectTimeoutMs(this._retryAttempt);
        connectTimeout = setTimeout(() => {
          if (this._isCurrentConn(conn, clientPeer, generation) && !conn.open) {
            const types = gathered.size ? [...gathered].join(',') : 'none';
            if (onLog) onLog(`[PeerJS] ICE timed out after ${timeoutMs}ms on attempt ${this._retryAttempt + 1} (candidates: ${types}) — closing and retrying`);
            if (onDiag) onDiag({ event: 'timeout', attempt: this._retryAttempt + 1, types: [...gathered] });
            this.conn = null;
            this._identSent = false;
            if (onStatus) onStatus('disconnected');
            conn.close();
            this._scheduleRetry();
          }
        }, timeoutMs);

        conn.on('open', () => {
          if (!this._isCurrentConn(conn, clientPeer, generation)) return;
          clearTimeout(connectTimeout);
          if (onLog) onLog('[PeerJS] DataChannel open');
          if (onDiag) onDiag({ event: 'open' });
          this._retryAttempt = 0;
          this._clearRetryTimer();
          const pc = conn.peerConnection;
          if (pc) {
            setTimeout(() => {
              pc.getStats().then(stats => {
                const vals = [...stats.values()];
                const pair = vals.find(r => r.type === 'candidate-pair' && r.nominated);
                if (pair) {
                  const loc = vals.find(r => r.id === pair.localCandidateId);
                  const rem = vals.find(r => r.id === pair.remoteCandidateId);
                  if (onLog) onLog(`[ICE] active path: local=${loc?.candidateType} remote=${rem?.candidateType}`);
                  if (onDiag) onDiag({ event: 'path', local: loc?.candidateType, remote: rem?.candidateType });
                  if (loc?.candidateType === 'relay' || rem?.candidateType === 'relay') {
                    if (onLog) onLog('[ICE] traffic is being relayed through a TURN server');
                  }
                }
              }).catch(() => {});
            }, 1500);

            // Create snapshot sub-channel (unordered, no retransmit)
            try {
              const snapChan = pc.createDataChannel('snapshot', { ordered: false, maxRetransmits: 0 });
              this._snapshotConn = snapChan;
              snapChan.onopen = () => {
                this._snapshotAvailable = true;
                if (onLog) onLog('[PeerJS] snapshot DataChannel open');
              };
              snapChan.onclose = () => {
                this._snapshotAvailable = false;
                if (onLog) onLog('[PeerJS] snapshot DataChannel closed');
              };
              snapChan.onerror = () => {
                this._snapshotAvailable = false;
              };
              // Inbound data on snapshot channel
              snapChan.onmessage = (event) => {
                if (!this._isCurrentConn(conn, clientPeer, generation)) return;
                if (typeof onData !== 'function') return;
                try {
                  const str = typeof event.data === 'string' ? event.data : new TextDecoder().decode(event.data);
                  // Resolve TOML string ids to display text once, here, so no
                  // console has to know which of its fields are localisable.
                  onData(localiseTree(JSON.parse(str)));
                } catch (e) {
                  if (onLog) onLog('[snapshot] bad message ' + e);
                }
              };
              if (onLog) onLog('[PeerJS] snapshot DataChannel created');
            } catch (e) {
              if (onLog) onLog('[PeerJS] failed to create snapshot channel:', e.message);
              this._snapshotAvailable = false;
            }
          }
          if (onStatus) onStatus('ready');
          if (typeof getIdent === 'function') {
            const ident = getIdent();
            if (ident && !this._identSent) {
              this._identSent = true;
              this.send('Identify', { token: ident.token, name: ident.name });
            }
          }
        });

        conn.on('data', raw => {
          if (!this._isCurrentConn(conn, clientPeer, generation)) return;
          if (typeof onData !== 'function') return;
          try {
            const str = typeof raw === 'string' ? raw : new TextDecoder().decode(raw);
            onData(localiseTree(JSON.parse(str)));
          } catch (e) {
            if (onLog) onLog('[client] bad message ' + e + ' ' + raw);
          }
        });

        conn.on('close', () => {
          if (!this._isCurrentConn(conn, clientPeer, generation)) return;
          clearTimeout(connectTimeout);
          if (onLog) onLog('[PeerJS] DataChannel closed');
          // Clean up snapshot channel
          this._snapshotAvailable = false;
          if (this._snapshotConn) {
            try { this._snapshotConn.close(); } catch (_) { /* already dead */ }
            this._snapshotConn = null;
          }
          // Reset so the next reopen re-sends Identify — the server restores
          // seat/rating from the token on every Identify, so this is what
          // makes reconnect actually resume the same seat.
          this._identSent = false;
          this.conn = null;
          if (onStatus) onStatus('disconnected');
          this._scheduleRetry();
        });

        conn.on('error', e => {
          if (!this._isCurrentConn(conn, clientPeer, generation)) return;
          clearTimeout(connectTimeout);
          if (onLog) onLog(`[PeerJS] connection error (type: ${e.type})`);
          if (onError) onError(e.type);
          // Don't treat this as a terminal state — fall back to the same
          // backoff retry loop instead of a permanent 'error' give-up.
          this._identSent = false;
          if (onStatus) onStatus('disconnected');
          this.conn = null;
          this._scheduleRetry();
        });
      };

      startConnect();
    });

    clientPeer.on('disconnected', () => {
      if (!this._isCurrentPeer(clientPeer, generation)) return;
      if (onLog) onLog('[PeerJS] signaling disconnected — reconnecting...');
      if (onStatus) onStatus('disconnected');
      clientPeer.reconnect();
    });

    clientPeer.on('error', e => {
      if (!this._isCurrentPeer(clientPeer, generation)) return;
      if (onLog) onLog(`[PeerJS] client error (type: ${e.type})`);
      if (onError) onError(e.type);
      // Signaling-layer errors are also retried with backoff rather than
      // giving up permanently — a slow persistent retry beats a dead end.
      if (onStatus) onStatus('disconnected');
      this._scheduleRetry();
    });
  }

  _isCurrentPeer(peer, generation) {
    return this._generation === generation && this.peer === peer;
  }

  _isCurrentConn(conn, peer, generation) {
    return this._isCurrentPeer(peer, generation) && this.conn === conn;
  }

  /** Schedule a reconnect attempt after an exponential backoff delay. */
  _scheduleRetry() {
    this._clearRetryTimer();
    const delay = nextBackoffDelay(this._retryAttempt);
    this._retryAttempt++;
    if (this._opts && this._opts.onLog) {
      this._opts.onLog(`[PeerJS] retrying in ${delay}ms (attempt ${this._retryAttempt})`);
    }
    this._retryTimer = setTimeout(() => this._reconnect(), delay);
  }

  _clearRetryTimer() {
    if (this._retryTimer) {
      clearTimeout(this._retryTimer);
      this._retryTimer = null;
    }
  }

  /** Tear down the current (possibly half-dead) peer and reconnect fresh. */
  _reconnect() {
    if (this.conn) {
      try { this.conn.close(); } catch (_) { /* already dead */ }
      this.conn = null;
    }
    if (this.peer) {
      try { this.peer.destroy(); } catch (_) { /* already dead */ }
      this.peer = null;
    }
    if (this._hostPeerId && this._opts) {
      this.connect(this._hostPeerId, this._opts);
    }
  }

  /**
   * User-visible "retry now" affordance: cancel any pending backoff wait and
   * attempt a reconnect immediately. No-op if already connected or if
   * connect() was never called.
   */
  retryNow() {
    if (this.connected) return;
    this._clearRetryTimer();
    this._reconnect();
  }

  send(type, data, deliveryClass) {
    if (!this.connected) return;
    // Snapshot-class messages ride the unordered channel when available
    if (deliveryClass === 'snapshot' && this._snapshotAvailable && this._snapshotConn) {
      const msg = data !== undefined ? { type, data } : { type };
      try {
        this._snapshotConn.send(JSON.stringify(msg));
        return;
      } catch (_) {
        // Fallback to reliable channel
      }
    }
    const msg = data !== undefined ? { type, data } : { type };
    this.conn.send(JSON.stringify(msg));
  }

  disconnect() {
    this._clearRetryTimer();
    this._hostPeerId = null;
    this._opts = null;
    this._snapshotAvailable = false;
    if (this._snapshotConn) {
      try { this._snapshotConn.close(); } catch (_) { /* already dead */ }
      this._snapshotConn = null;
    }
    if (this.conn) {
      this.conn.close();
      this.conn = null;
    }
    if (this.peer) {
      this.peer.destroy();
      this.peer = null;
    }
  }
}

export const connectionManager = new ConnectionManager();

if (typeof window !== 'undefined') {
  window.connectionManager = connectionManager;
  window.fetchIceServers = fetchIceServers;
  window.probeTurnRelay = probeTurnRelay;
}
