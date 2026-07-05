// BroadcastChannel-backed PeerJS shim for smoke testing.
// Injected via Playwright addInitScript — replaces window.Peer before any
// page scripts run. Routes messages between pages via BroadcastChannel
// (same-origin only, which is fine since all test pages share localhost:3000).
//
// Dispatches 'wasm-ready' on window (and sets window.__wasmReady = true) once
// BOTH the host peer has opened AND server.html has fired PhoenixReady.

(function () {
  'use strict';

  const CHANNEL = 'peerjs-shim';
  const registry = new Map(); // peerId -> Peer instance
  const offlinePairs = new Set();
  const offlineEndpoints = new Set();
  let bc = null;

  function getChannel() {
    if (!bc) {
      bc = new BroadcastChannel(CHANNEL);
      bc.onmessage = function (ev) {
        if (ev.data && ev.data.t === 'control') {
          handleControl(ev.data);
          return;
        }
        var peer = registry.get(ev.data.to);
        if (peer) peer._recv(ev.data);
      };
    }
    return bc;
  }

  function genId() {
    return Array.from(crypto.getRandomValues(new Uint8Array(8)))
      .map(function (b) { return b.toString(16).padStart(2, '0'); })
      .join('');
  }

  function pairKey(peerIdA, peerIdB) {
    return [peerIdA, peerIdB].sort().join('\0');
  }

  function isOffline(localId, remoteId) {
    return offlinePairs.has(pairKey(localId, remoteId)) ||
      offlineEndpoints.has(localId) ||
      offlineEndpoints.has(remoteId);
  }

  function closeLocalSide(localId, remoteId) {
    var peer = registry.get(localId);
    if (!peer) return;
    peer._sever(remoteId);
  }

  function applySever(peerIdA, peerIdB) {
    offlinePairs.add(pairKey(peerIdA, peerIdB));
    // Reconnect creates a fresh client peer id; holding the stable endpoint
    // offline keeps retries blocked until the test explicitly revives it.
    offlineEndpoints.add(peerIdA);
    offlineEndpoints.add(peerIdB);
    closeLocalSide(peerIdA, peerIdB);
    closeLocalSide(peerIdB, peerIdA);
  }

  function applyRevive(peerIdA, peerIdB) {
    offlinePairs.delete(pairKey(peerIdA, peerIdB));
    offlineEndpoints.delete(peerIdA);
    offlineEndpoints.delete(peerIdB);
  }

  function broadcastControl(action, peerIdA, peerIdB) {
    getChannel().postMessage({ t: 'control', action: action, peerIdA: peerIdA, peerIdB: peerIdB });
  }

  function handleControl(msg) {
    if (msg.action === 'sever') {
      applySever(msg.peerIdA, msg.peerIdB);
    } else if (msg.action === 'revive') {
      applyRevive(msg.peerIdA, msg.peerIdB);
    }
  }

  // ── Minimal event emitter ──────────────────────────────────────────────────

  function Emitter() { this._h = {}; }
  Emitter.prototype.on = function (ev, fn) {
    if (!this._h[ev]) this._h[ev] = [];
    this._h[ev].push(fn);
    return this;
  };
  Emitter.prototype._emit = function (ev) {
    var args = Array.prototype.slice.call(arguments, 1);
    (this._h[ev] || []).forEach(function (fn) { fn.apply(null, args); });
  };

  // ── DataChannel shim ───────────────────────────────────────────────────────
  // Represents a sub-channel on the same RTCPeerConnection (snapshot channel).

  function DataChannel(localId, remoteId, label, opts) {
    Emitter.call(this);
    this.label = label;
    this._lid = localId;
    this._rid = remoteId;
    this.readyState = 'connecting';
    this.ordered = opts ? opts.ordered !== false : true;
    this.maxRetransmits = opts ? opts.maxRetransmits : null;
    this._dropRate = 0;
    this.onopen = null;
    this.onclose = null;
    this.onerror = null;
    this.onmessage = null;

    var self = this;
    Promise.resolve().then(function () {
      self.readyState = 'open';
      self._emit('open');
      if (typeof self.onopen === 'function') self.onopen();
      // Notify remote side about this DataChannel
      getChannel().postMessage({
        t: 'datachannel', from: localId, to: remoteId,
        label: label, ordered: self.ordered, maxRetransmits: self.maxRetransmits,
      });
    });
  }
  DataChannel.prototype = Object.create(Emitter.prototype);
  DataChannel.prototype.constructor = DataChannel;

  DataChannel.prototype.send = function (data) {
    if (this.readyState !== 'open') return;
    if (this._dropRate > 0 && Math.random() < this._dropRate) return;
    getChannel().postMessage({
      t: 'data', from: this._lid, to: this._rid,
      data: data, channel: this.label,
    });
  };

  DataChannel.prototype.close = function () {
    this.readyState = 'closed';
    this._emit('close');
    if (typeof this.onclose === 'function') this.onclose();
  };

  // ── Connection shim ────────────────────────────────────────────────────────

  function Connection(localId, remoteId) {
    Emitter.call(this);
    this.peer = remoteId;   // PeerJS API: conn.peer is the remote peer's ID
    this._lid = localId;
    this._rid = remoteId;
    this.open = false;
    // DataChannel sub-channels keyed by label (e.g. 'snapshot')
    this._dataChannels = new Map();
    this.peerConnection = {
      createDataChannel: function (label, opts) {
        var dc = new DataChannel(localId, remoteId, label, opts);
        return dc;
      },
      ondatachannel: null,
    };
  }
  Connection.prototype = Object.create(Emitter.prototype);
  Connection.prototype.constructor = Connection;

  Connection.prototype.send = function (data) {
    if (isOffline(this._lid, this._rid)) return;
    getChannel().postMessage({ t: 'data', from: this._lid, to: this._rid, data: data });
  };

  Connection.prototype.close = function () {
    this.open = false;
    // Close all sub-channels
    this._dataChannels.forEach(function (dc) { dc.close(); });
    this._dataChannels.clear();
    getChannel().postMessage({ t: 'close', from: this._lid, to: this._rid });
    this._emit('close');
  };

  Connection.prototype._open = function () {
    this.open = true;
    this._emit('open');
  };

  Connection.prototype._data = function (d) {
    this._emit('data', d);
  };

  Connection.prototype._close = function () {
    this.open = false;
    this._dataChannels.forEach(function (dc) { dc.close(); });
    this._dataChannels.clear();
    this._emit('close');
  };

  /**
   * Register a DataChannel on this Connection and fire ondatachannel.
   * Called by the remote side when a 'datachannel' control message arrives.
   */
  Connection.prototype._addDataChannel = function (label, opts) {
    var dc = new DataChannel(this._lid, this._rid, label, opts);
    this._dataChannels.set(label, dc);
    var self = this;
    Promise.resolve().then(function () {
      dc.readyState = 'open';
      dc._emit('open');
      if (typeof self.peerConnection.ondatachannel === 'function') {
        self.peerConnection.ondatachannel({ channel: dc });
      }
    });
    return dc;
  };

  // ── wasm-ready signalling ─────────────────────────────────────────────────
  //
  // __wasmReady is set when BOTH:
  //   1. The host peer has opened (so readHostPeerId can return a peer ID), AND
  //   2. PhoenixReady has fired — dispatched by server.html's finishInit() after
  //      the async config preload completes and wasm_init() has been called.
  //
  // Previously this used TrunkApplicationStarted, but that fires before the
  // async map/entity config fetch sequence completes, causing Welcome timeouts.

  var _peerOpened = false;
  var _phoenixReady = false;
  var _wasmReadyFired = false;

  function _maybeDispatch() {
    if (_peerOpened && _phoenixReady && !_wasmReadyFired) {
      _wasmReadyFired = true;
      window.__wasmReady = true;
      window.dispatchEvent(new CustomEvent('wasm-ready'));
    }
  }

  // PhoenixReady is dispatched by server.html's finishInit() after the full
  // async preload sequence and wasm_init() call are complete.
  window.addEventListener('PhoenixReady', function () {
    _phoenixReady = true;
    _maybeDispatch();
  });

  // ── Peer shim ─────────────────────────────────────────────────────────────

  function Peer(id, _opts) {
    Emitter.call(this);
    // PeerJS supports both new Peer(id, opts) and new Peer(opts).
    // When the first argument is an object it is the options bag, not an ID.
    // Using an object as a Map key breaks BroadcastChannel registry lookups
    // because structured-clone produces a new reference on the receiving side.
    this.id = (typeof id === 'string' && id) ? id : genId();
    this._conns = new Map(); // remoteId -> Connection
    registry.set(this.id, this);
    getChannel(); // ensure BroadcastChannel is listening

    var self = this;
    Promise.resolve().then(function () {
      self._emit('open', self.id);
      _peerOpened = true;
      // If PhoenixReady already fired before the peer opened, dispatch now.
      if (_phoenixReady) _maybeDispatch();
    });
  }
  Peer.prototype = Object.create(Emitter.prototype);
  Peer.prototype.constructor = Peer;

  Peer.prototype.connect = function (remoteId) {
    var conn = new Connection(this.id, remoteId);
    this._conns.set(remoteId, conn);
    if (isOffline(this.id, remoteId)) return conn;
    getChannel().postMessage({ t: 'connect', from: this.id, to: remoteId });
    return conn;
  };

  Peer.prototype.reconnect = function () {
    var self = this;
    Promise.resolve().then(function () { self._emit('open', self.id); });
  };

  Peer.prototype.destroy = function () {
    registry.delete(this.id);
    if (bc) { bc.close(); bc = null; }
  };

  // ── Test-only kill/revive support (issue #614) ─────────────────────────────
  //
  // Simulates a silently-dropped DataChannel — e.g. a phone's radio sleeping
  // mid-game — without destroying the underlying Peer (signaling channel) on
  // either side. Real WebRTC failure modes like this fire the DataChannel's
  // 'close'/'error' event on BOTH ends without either side having called
  // conn.close() itself, which is exactly what connection-manager.js's
  // reconnect-with-backoff logic needs to observe to kick in.
  //
  // `_sever(remoteId)` fires `_close()` locally on this Peer object's
  // Connection to `remoteId` (if any) WITHOUT posting a 'close' broadcast
  // message. A BroadcastChannel control message asks every browser context to
  // apply the same local close, so both sides observe the DataChannel close.
  Peer.prototype._sever = function (remoteId) {
    var conn = this._conns.get(remoteId);
    if (conn) {
      conn._close();
      this._conns.delete(remoteId);
    }
  };

  // Exposed as a small test-only global API rather than bloating the public
  // Peer/Connection surface real PeerJS exposes. Tests reach in via
  // `window.__peerjsShim.severConnection(peerIdA, peerIdB)`.
  window.__peerjsShim = {
    /**
     * Sever the DataChannel between two peer IDs on both ends, simulating a
     * dropped connection that neither side initiated. The pair stays offline
     * until reviveConnection() so the client's backoff retry cannot race ahead
     * before the test observes and clicks the retry control.
     */
    severConnection: function (peerIdA, peerIdB) {
      applySever(peerIdA, peerIdB);
      broadcastControl('sever', peerIdA, peerIdB);
    },

    /**
     * Re-enable the previously severed link. Existing blocked Connection
     * objects stay closed/half-open; the client must retry to create a fresh
     * DataChannel, matching the real user-visible "retry now" path.
     */
    reviveConnection: function (peerIdA, peerIdB) {
      applyRevive(peerIdA, peerIdB);
      broadcastControl('revive', peerIdA, peerIdB);
    },

    /**
     * Get all registered DataChannels for debugging/inspection.
     */
    _dataChannels: function () {
      var result = {};
      registry.forEach(function (peer, peerId) {
        peer._conns.forEach(function (conn, remoteId) {
          conn._dataChannels.forEach(function (dc, label) {
            result[peerId + '->' + remoteId + ':' + label] = {
              readyState: dc.readyState,
              ordered: dc.ordered,
              maxRetransmits: dc.maxRetransmits,
              dropRate: dc._dropRate,
            };
          });
        });
      });
      return result;
    },
  };

  // Close all open connections cleanly when the page is being torn down so
  // remote peers (the host) receive a 'close' event and can drive the
  // wasm_player_disconnected lifecycle. Without this, page.close() in
  // Playwright simply kills the BroadcastChannel listener, leaving the host
  // with stale session state and consoles still "occupied" by the gone peer.
  window.addEventListener('pagehide', function () {
    registry.forEach(function (peer) {
      peer._conns.forEach(function (conn) {
        if (conn.open) {
          try {
            getChannel().postMessage({ t: 'close', from: conn._lid, to: conn._rid });
          } catch (_) { /* channel already closed */ }
        }
      });
    });
  });

  Peer.prototype._recv = function (msg) {
    var self = this;
    switch (msg.t) {
      case 'connect': {
        if (isOffline(this.id, msg.from)) break;
        var inConn = new Connection(this.id, msg.from);
        this._conns.set(msg.from, inConn);
        // Send accept first, then surface to caller so they can register handlers
        // before open fires.
        getChannel().postMessage({ t: 'accept', from: this.id, to: msg.from });
        this._emit('connection', inConn);
        // Open fires on next microtask so handlers registered in 'connection'
        // callback are already in place.
        Promise.resolve().then(function () { inConn._open(); });
        break;
      }
      case 'accept': {
        if (isOffline(this.id, msg.from)) break;
        var outConn = this._conns.get(msg.from);
        if (outConn) Promise.resolve().then(function () { outConn._open(); });
        break;
      }
      case 'datachannel': {
        if (isOffline(this.id, msg.from)) break;
        var conn = this._conns.get(msg.from);
        if (conn) {
          conn._addDataChannel(msg.label, {
            ordered: msg.ordered,
            maxRetransmits: msg.maxRetransmits,
          });
        }
        break;
      }
      case 'data': {
        if (isOffline(this.id, msg.from)) break;
        if (msg.channel && msg.channel !== 'reliable') {
          // Route to specific DataChannel
          var dcConn = this._conns.get(msg.from);
          if (dcConn) {
            var dc = dcConn._dataChannels.get(msg.channel);
            if (dc && typeof dc.onmessage === 'function') {
              dc.onmessage({ data: msg.data });
            }
          }
        } else {
          var dataConn = this._conns.get(msg.from);
          if (dataConn) dataConn._data(msg.data);
        }
        break;
      }
      case 'close': {
        var closeConn = this._conns.get(msg.from);
        if (closeConn) {
          closeConn._close();
          this._conns.delete(msg.from);
        }
        break;
      }
    }
  };

  window.Peer = Peer;
})();
