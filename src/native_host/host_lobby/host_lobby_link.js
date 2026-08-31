// Host-lobby link — the LAST script in the native host's lobby document
// (issue #1325).
//
// A module, and last in <body>, for the same two reasons pane_link.js is:
// it `import`s from gui/ (which a classic script cannot), and it must run
// after the document's own markup is parsed so `renderHostLobby` has elements
// to write into.
//
// It is the whole of the wiring, and there is deliberately very little of it —
// every decision it makes is one the WEB host already makes with the same
// three modules:
//
//   gui/host-channel.js       resolve string ids in the payload (issue #949)
//   gui/host-lobby-view.js    payload + previous phase -> view model (#1229)
//   gui/host-lobby-render.js  view model -> the DOM inside #lobby-panel (#1325)
//
// If this file ever grows a fourth decision, that decision has escaped the
// shared path and belongs back in one of those modules instead.
import './gui/strings-boot.js';
import { t, localiseTree, applyToDom } from './gui/strings.js';
import { localiseHostPayload } from './gui/host-channel.js';
import { hostLobbyViewModel } from './gui/host-lobby-view.js';
import { renderHostLobby } from './gui/host-lobby-render.js';

// The static `data-i18n` markup — "CREW", "CONNECTED", the awaiting-selection
// badge — is substituted once here, exactly as server.html's own module island
// does it. Everything data-driven is resolved per render by `t` below.
applyToDom(document);

// `hostLobbyViewModel`'s second argument is the phase seen on the previous
// call: it is what makes the Loading -> InProgress edge distinguishable from
// simply being InProgress. The web host keeps it in `_lobbyPrevPhase`; this is
// the same variable under a different roof. Nothing on this surface acts on the
// edges it drives (there is no audio and no loading overlay here), but the view
// model is shared, so it is fed honestly rather than fed a constant.
let prevPhase = '';

const strings = { t, localiseTree };

window.__phoenixHostLobby.render = function (json, revealChrome) {
  // The localisation boundary, in the same place the web host puts it: a lobby
  // payload is built from authored data and can carry string ids (a world's
  // `[global] title`), and resolving them once at the edge is what stopped
  // `world.combat_test.global.title` reaching #lobby-title (issue #949).
  const localised = localiseHostPayload(json, strings);
  let payload;
  try {
    payload = JSON.parse(localised);
  } catch (e) {
    console.warn('[host-lobby] bad lobby state json', e);
    return;
  }
  const vm = hostLobbyViewModel(payload, prevPhase);
  prevPhase = payload.phase;
  renderHostLobby(document, vm, t, { revealChrome });
};

// Anything the host pushed while this island was still loading renders now.
window.__phoenixHostLobby.paint();
