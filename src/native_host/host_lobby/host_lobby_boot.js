// Host-lobby boot — the FIRST script in the native host's lobby document
// (issue #1325).
//
// Native-side glue, not client code: it lives under src/native_host/host_lobby/
// and is injected into a document assembled from the host page's own lobby
// markup, so gui/ stays exactly as server.html loads it.
//
// It has to be a CLASSIC script at the top of <head>, for the same reason
// pane_boot.js does: it installs the two host->page entry points and the
// page->host queue BEFORE any module evaluates. The host starts pushing the
// moment the document reports itself loaded, and "loaded" is not "the module
// island has run" — a push that lands in the gap must be kept, not thrown away.
//
// WHAT IT DOES NOT DO, and why that is worth saying:
//
//   - It does not seed an identity. There is none. This surface holds no
//     session token, and what it sends over the page->host queue is not a
//     participant's ClientMessage but the host OPERATOR's own presses — a
//     scenario, a hull, an AI launch (issue #1328), a monitor for the
//     viewscreen (issue #1330) — every one of them decoded by the single
//     vocabulary native_host::host_lobby::HostLobbyRecord. A press asks the
//     host to arbitrate its own lobby or rearrange its own screens; nothing
//     here is anything a participant may say. The queue the shim installs
//     beside these entry points is what carries them — installed in #1325
//     before anything rode it, so the bridge would have one shape rather than
//     grow a second one later. The one control that sends nothing at all is
//     the QR toggle (issue #1329), which acts on this document's own DOM.
//   - It does not replace requestAnimationFrame. pane_boot.js has to, because
//     the client page's render loop is `scheduleRender()` -> rAF and an
//     offscreen Ultralight view only services rAF as part of a rendering
//     update, which deadlocks a quiet page. This document has no such loop:
//     every repaint is driven synchronously from a host push, inside the
//     host's own evaluate_script. There is nothing to starve.
(function () {
  // The last state each side of the bridge sent, and the renderer the module
  // island installs. Kept as STATE rather than as a queue: a lobby payload is a
  // snapshot of the whole lobby, so an older one has nothing to say that the
  // newest one does not — which is also why the host side collapses to
  // latest-wins instead of carrying a backlog.
  var lobby = {
    payload: null,
    revealChrome: false,
    // The bridge's monitor row (issue #1330), or null on a host that has not
    // pushed one. Null is the honest starting value and the honest value
    // forever on any surface that never gets one: the row renders only when a
    // native bridge has reported its monitors.
    layout: null,
    render: null,
    // The join panel's half of the same arrangement (issue #1329). `join` is
    // the last invitation pushed — a snapshot, like the payload above.
    // `qrPending` is NOT a snapshot: it counts presses of a phone's QR toggle
    // that have not been applied yet, because two presses are two flips and
    // collapsing them would turn a double-press into a single one.
    join: null,
    qrPending: 0,
    renderJoin: null,
    // The scenario picker's half (issue #1328). A snapshot too: it carries the
    // whole catalogue, whatever the arbiter has locked, and whether a world has
    // landed and closed the picker for good.
    scenario: null,
    renderScenario: null,
  };
  window.__phoenixHostLobby = lobby;

  lobby.paint = function () {
    if (!lobby.render || lobby.payload === null) return;
    try {
      lobby.render(lobby.payload, lobby.revealChrome, lobby.layout);
    } catch (e) {
      // A throw here would propagate out of the host's evaluate_script and be
      // read as a failed push, which the host would retry with the same
      // payload, forever. Report and keep the surface alive instead.
      console.error('[host-lobby] render failed', e);
    }
  };

  // Host -> page: one encoded LobbyStatePayload, the same JSON the web host
  // consumes on its "lobby" channel.
  window.__phoenixHostLobbyApply = function (json) {
    lobby.payload = json;
    lobby.paint();
  };

  // Host -> page: whether to render the lobby chrome even though the phase
  // would hide it (native_host::host_lobby::reveal). The bridge's only
  // primitive is a call with a single STRING argument, so the flag arrives as
  // 'true' / 'false' rather than as a boolean.
  window.__phoenixHostLobbyReveal = function (flag) {
    lobby.revealChrome = String(flag) === 'true';
    lobby.paint();
  };

  // Host -> page: the bridge's monitor row, encoded BridgeLayoutPayload
  // (native_host::host_lobby::layout). Held as the newest snapshot, like the
  // lobby payload above and for the same reason — an older roster has nothing
  // to say the newest one does not.
  window.__phoenixHostLobbyLayout = function (json) {
    lobby.layout = json;
    lobby.paint();
  };

  lobby.paintJoin = function () {
    if (!lobby.renderJoin) return;
    try {
      var pending = lobby.qrPending;
      // Cleared BEFORE the call, not after: a throw inside renderJoin would
      // otherwise leave the presses queued and re-apply them on the next push,
      // flipping the panel for reasons nobody in the room can see.
      lobby.qrPending = 0;
      lobby.renderJoin(lobby.join, pending);
    } catch (e) {
      console.error('[host-lobby] join render failed', e);
    }
  };

  // Host -> page: one encoded JoinInvite (native_host::host_lobby::join) —
  // the crew's code, and where a phone that scans it should go.
  window.__phoenixHostLobbyJoin = function (json) {
    lobby.join = json;
    lobby.paintJoin();
  };

  lobby.paintScenario = function () {
    if (!lobby.renderScenario || lobby.scenario === null) return;
    try {
      lobby.renderScenario(lobby.scenario);
    } catch (e) {
      // Same reason lobby.paint() swallows: a throw out of here propagates out
      // of the host's evaluate_script, is read as a failed push, and is retried
      // with the same payload forever.
      console.error('[host-lobby] scenario render failed', e);
    }
  };

  // Host -> page: one encoded ScenarioPanelPayload
  // (native_host::host_lobby::scenario) — the catalogue this host publishes,
  // what its arbiter has locked, and whether a world has closed the picker.
  window.__phoenixHostLobbyScenario = function (json) {
    lobby.scenario = json;
    lobby.paintScenario();
  };

  // Host -> page: somebody pressed the QR toggle on a phone. No argument: the
  // panel's visibility lives in the DOM (gui/host-qr.js reads #overlay), so
  // there is nothing for the host to have an opinion about.
  window.__phoenixHostLobbyQrToggle = function () {
    lobby.qrPending += 1;
    lobby.paintJoin();
  };
})();
