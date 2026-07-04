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

export class ConnectionManager {
  constructor() {
    this.conn = null;
    this.peer = null;
    this._identSent = false;
  }

  get connected() {
    return this.conn !== null && this.conn.open;
  }

  connect(hostPeerId, { iceServers, onData, onStatus, onError, onLog, getIdent } = {}) {
    const Peer = typeof window !== 'undefined' ? window.Peer : null;
    if (!Peer || !hostPeerId) return;

    this._identSent = false;
    const clientPeer = new Peer({ config: { iceServers: iceServers || defaultIceServers() } });
    this.peer = clientPeer;

    if (onLog) onLog('[PeerJS] connecting to host peer ID: ' + hostPeerId);

    clientPeer.on('open', () => {
      if (onLog) onLog('[PeerJS] client peer open — starting DataChannel connect');
      let connectTimeout;
      let connectAttempts = 0;

      const startConnect = () => {
        if (onLog) onLog(`[PeerJS] connect attempt ${connectAttempts + 1}—`);
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
          if (conn && !conn.open) {
            if (onLog) onLog(`[PeerJS] ICE timed out on attempt ${connectAttempts + 1} — closing and retrying`);
            conn.close();
            this.conn = null;
            connectAttempts++;
            if (connectAttempts >= 3) {
              if (onLog) onLog('[PeerJS] giving up after 3 failed ICE attempts');
              if (onStatus) onStatus('error');
              return;
            }
            startConnect();
          }
        }, 8000);

        conn.on('open', () => {
          clearTimeout(connectTimeout);
          if (onLog) onLog('[PeerJS] DataChannel open');
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
          if (typeof onData !== 'function') return;
          try {
            const str = typeof raw === 'string' ? raw : new TextDecoder().decode(raw);
            onData(JSON.parse(str));
          } catch (e) {
            if (onLog) onLog('[client] bad message ' + e + ' ' + raw);
          }
        });

        conn.on('close', () => {
          clearTimeout(connectTimeout);
          if (onLog) onLog('[PeerJS] DataChannel closed');
          if (onStatus) onStatus('disconnected');
          this.conn = null;
        });

        conn.on('error', e => {
          clearTimeout(connectTimeout);
          if (onLog) onLog(`[PeerJS] connection error (type: ${e.type})`);
          if (onStatus) onStatus('error');
          if (onError) onError(e.type);
        });
      };

      startConnect();
    });

    clientPeer.on('disconnected', () => {
      if (onLog) onLog('[PeerJS] signaling disconnected — reconnecting...');
      if (onStatus) onStatus('disconnected');
      clientPeer.reconnect();
    });

    clientPeer.on('error', e => {
      if (onLog) onLog(`[PeerJS] client error (type: ${e.type})`);
      if (onStatus) onStatus('error');
      if (onError) onError(e.type);
    });
  }

  send(type, data) {
    if (!this.connected) return;
    const msg = data !== undefined ? { type, data } : { type };
    this.conn.send(JSON.stringify(msg));
  }

  disconnect() {
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
