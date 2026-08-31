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
//   - It does not seed an identity. There is none. This surface is READ-ONLY in
//     this slice and holds no session token: it renders the lobby the host is
//     already broadcasting and sends nothing back. The page->host queue below
//     is plumbing for the slices that will (a QR overlay, settings, layout
//     rows), and it is installed now so the bridge has one shape rather than
//     growing a second one later.
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
    render: null,
  };
  window.__phoenixHostLobby = lobby;

  lobby.paint = function () {
    if (!lobby.render || lobby.payload === null) return;
    try {
      lobby.render(lobby.payload, lobby.revealChrome);
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
})();
