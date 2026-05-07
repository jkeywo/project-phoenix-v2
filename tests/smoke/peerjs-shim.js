// BroadcastChannel-backed PeerJS shim for smoke testing.
// Injected via Playwright addInitScript — replaces window.Peer before any
// page scripts run. Routes messages between pages via BroadcastChannel
// (same-origin only, which is fine since all test pages share localhost:3000).
//
// Dispatches 'wasm-ready' on window (and sets window.__wasmReady = true) once
// BOTH the host peer has opened AND TrunkApplicationStarted has fired — the
// setTimeout guarantees startPhoenix() has run before the signal goes out.

(function () {
  'use strict';

  const CHANNEL = 'peerjs-shim';
  const registry = new Map(); // peerId -> Peer instance
  let bc = null;

  function getChannel() {
    if (!bc) {
      bc = new BroadcastChannel(CHANNEL);
      bc.onmessage = function (ev) {
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

  // ── Connection shim ────────────────────────────────────────────────────────

  function Connection(localId, remoteId) {
    Emitter.call(this);
    this.peer = remoteId;   // PeerJS API: conn.peer is the remote peer's ID
    this._lid = localId;
    this._rid = remoteId;
    this.open = false;
  }
  Connection.prototype = Object.create(Emitter.prototype);
  Connection.prototype.constructor = Connection;

  Connection.prototype.send = function (data) {
    getChannel().postMessage({ t: 'data', from: this._lid, to: this._rid, data: data });
  };

  Connection.prototype.close = function () {
    this.open = false;
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
    this._emit('close');
  };

  // ── wasm-ready signalling ─────────────────────────────────────────────────

  var _peerOpened = false;
  var _trunkFired = false;
  var _wasmReadyFired = false;

  function _maybeDispatch() {
    if (_peerOpened && _trunkFired && !_wasmReadyFired) {
      _wasmReadyFired = true;
      window.__wasmReady = true;
      window.dispatchEvent(new CustomEvent('wasm-ready'));
    }
  }

  // TrunkApplicationStarted fires after the WASM binary is loaded.
  // We use setTimeout(0) so startPhoenix() (registered after us) runs first.
  window.addEventListener('TrunkApplicationStarted', function () {
    _trunkFired = true;
    setTimeout(_maybeDispatch, 0);
  });

  // ── Peer shim ─────────────────────────────────────────────────────────────

  function Peer(id) {
    Emitter.call(this);
    this.id = id || genId();
    this._conns = new Map(); // remoteId -> Connection
    registry.set(this.id, this);
    getChannel(); // ensure BroadcastChannel is listening

    var self = this;
    Promise.resolve().then(function () {
      self._emit('open', self.id);
      _peerOpened = true;
      // Handle the case where TrunkApplicationStarted already fired before
      // the peer opened (rare, but possible if WASM was cached).
      if (_trunkFired) setTimeout(_maybeDispatch, 0);
    });
  }
  Peer.prototype = Object.create(Emitter.prototype);
  Peer.prototype.constructor = Peer;

  Peer.prototype.connect = function (remoteId) {
    var conn = new Connection(this.id, remoteId);
    this._conns.set(remoteId, conn);
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

  Peer.prototype._recv = function (msg) {
    var self = this;
    switch (msg.t) {
      case 'connect': {
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
        var outConn = this._conns.get(msg.from);
        if (outConn) Promise.resolve().then(function () { outConn._open(); });
        break;
      }
      case 'data': {
        var dataConn = this._conns.get(msg.from);
        if (dataConn) dataConn._data(msg.data);
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
