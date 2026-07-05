export function defaultIceServers() {
  return [
    { urls: 'stun:stun.l.google.com:19302' },
    { urls: 'stun:stun1.l.google.com:19302' },
    { urls: 'turn:openrelay.metered.ca:80',  username: 'openrelayproject', credential: 'openrelayproject' },
    { urls: 'turn:openrelay.metered.ca:443', username: 'openrelayproject', credential: 'openrelayproject' },
    { urls: 'turn:openrelay.metered.ca:443', username: 'openrelayproject', credential: 'openrelayproject', transport: 'tcp' },
  ];
}

export async function fetchIceServers() {
  const base = defaultIceServers();
  try {
    const r = await fetch('https://phoenix-turn-credentials.project-phoenix.workers.dev');
    if (r.ok) {
      const extra = await r.json();
      console.log(`[ICE] Metered.ca returned ${extra.length} server(s) — appending to base list`);
      return [...base, ...extra];
    }
    console.warn('[ICE] Metered.ca fetch returned', r.status, '— using OpenRelay only');
  } catch (e) {
    console.warn('[ICE] Metered.ca fetch failed (placeholder or network error) — using OpenRelay only:', e.message);
  }
  return base;
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

export class ConnectionManager {
  constructor() {
    this.conn = null;
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

  connect(hostPeerId, { iceServers, onData, onStatus, onError, onLog, getIdent } = {}) {
    const Peer = typeof window !== 'undefined' ? window.Peer : null;
    if (!Peer || !hostPeerId) return;

    // Stash for retryNow()/backoff-triggered reconnects, which re-run this
    // same setup against a fresh Peer.
    this._hostPeerId = hostPeerId;
    this._opts = { iceServers, onData, onStatus, onError, onLog, getIdent };
    this._identSent = false;
    this._clearRetryTimer();
    const generation = ++this._generation;

    const clientPeer = new Peer({ config: { iceServers: iceServers || defaultIceServers() } });
    this.peer = clientPeer;
    this._clientPeer = clientPeer;

    if (onLog) onLog('[PeerJS] connecting to host peer ID: ' + hostPeerId);

    clientPeer.on('open', () => {
      if (!this._isCurrentPeer(clientPeer, generation)) return;
      if (onLog) onLog('[PeerJS] client peer open — starting DataChannel connect');
      let connectTimeout;

      const startConnect = () => {
        if (!this._isCurrentPeer(clientPeer, generation)) return;
        if (onLog) onLog(`[PeerJS] connect attempt ${this._retryAttempt + 1}—`);
        if (onStatus) onStatus('connecting');
        const conn = clientPeer.connect(hostPeerId, { reliable: true });
        this.conn = conn;

        const pc = conn.peerConnection;
        if (pc) {
          pc.addEventListener('iceconnectionstatechange', () => {
            if (onLog) onLog(`[ICE] state — ${pc.iceConnectionState}`);
          });
          pc.addEventListener('icegatheringstatechange', () => {
            if (onLog) onLog(`[ICE] gathering — ${pc.iceGatheringState}`);
          });
        }

        connectTimeout = setTimeout(() => {
          if (this._isCurrentConn(conn, clientPeer, generation) && !conn.open) {
            if (onLog) onLog(`[PeerJS] ICE timed out on attempt ${this._retryAttempt + 1} — closing and retrying`);
            this.conn = null;
            this._identSent = false;
            if (onStatus) onStatus('disconnected');
            conn.close();
            this._scheduleRetry();
          }
        }, 8000);

        conn.on('open', () => {
          if (!this._isCurrentConn(conn, clientPeer, generation)) return;
          clearTimeout(connectTimeout);
          if (onLog) onLog('[PeerJS] DataChannel open');
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
                  if (loc?.candidateType === 'relay' || rem?.candidateType === 'relay') {
                    if (onLog) onLog('[ICE] traffic is being relayed through a TURN server');
                  }
                }
              }).catch(() => {});
            }, 1500);
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
            onData(JSON.parse(str));
          } catch (e) {
            if (onLog) onLog('[client] bad message ' + e + ' ' + raw);
          }
        });

        conn.on('close', () => {
          if (!this._isCurrentConn(conn, clientPeer, generation)) return;
          clearTimeout(connectTimeout);
          if (onLog) onLog('[PeerJS] DataChannel closed');
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

  send(type, data) {
    if (!this.connected) return;
    const msg = data !== undefined ? { type, data } : { type };
    this.conn.send(JSON.stringify(msg));
  }

  disconnect() {
    this._clearRetryTimer();
    this._hostPeerId = null;
    this._opts = null;
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
}
