/**
 * gui/bg-raf-keepalive.js — Unthrottled requestAnimationFrame in hidden tabs
 *
 * Browsers cap requestAnimationFrame to ~1 Hz once a document is hidden
 * (tab backgrounded, window minimized). Fine for a page that's idling, but
 * several consoles run continuous rAF loops that other crew are actively
 * relying on — the radar sweep (radar-widget.js), the helm joystick decay
 * loop, the repair countdown timers — and those need to keep ticking at
 * full rate even when this console's browser tab isn't the focused one.
 *
 * Same MessageChannel trick as the background-tab sim heartbeat in
 * server.html (MessageChannel tasks aren't subject to the background-tab
 * throttling), applied to rAF instead of Bevy's WASM setTimeout cadence.
 * Delegates straight to native rAF while the document is visible so
 * on-screen consoles keep normal vsync-aligned timing.
 *
 * Must load before any script that calls requestAnimationFrame.
 */
(function () {
  'use strict';
  var _nativeRAF = window.requestAnimationFrame.bind(window);
  var _nativeCAF = window.cancelAnimationFrame.bind(window);
  var FRAME_MS = 1000 / 60;

  var _mc = new MessageChannel();
  var _queue = [];           // [{ cb, id }] due this pump
  var _nextId = 0x40000000;  // fake ids parked well above native rAF ids
  var _active = {};
  var _scheduled = false;
  var _lastRun = 0;

  function _schedule() {
    if (!_scheduled) {
      _scheduled = true;
      _mc.port1.postMessage('');
    }
  }

  _mc.port2.onmessage = function () {
    _scheduled = false;
    var now = performance.now();
    if (now - _lastRun < FRAME_MS) { _schedule(); return; }
    _lastRun = now;
    var due = _queue;
    _queue = [];
    for (var i = 0; i < due.length; i++) {
      var entry = due[i];
      if (!_active[entry.id]) continue; // cancelled before it fired
      delete _active[entry.id];
      try { entry.cb(now); } catch (e) { console.error(e); }
    }
  };

  window.requestAnimationFrame = function (cb) {
    if (!document.hidden) return _nativeRAF(cb);
    var id = ++_nextId;
    _active[id] = true;
    _queue.push({ cb: cb, id: id });
    _schedule();
    return id;
  };

  window.cancelAnimationFrame = function (id) {
    if (_active[id]) { delete _active[id]; return; }
    _nativeCAF(id);
  };
})();
