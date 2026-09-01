// Host-lobby link — the LAST script in the native host's lobby document
// (issue #1325).
//
// A module, and last in <body>, for the same two reasons pane_link.js is:
// it `import`s from gui/ (which a classic script cannot), and it must run
// after the document's own markup is parsed so `renderHostLobby` has elements
// to write into.
//
// It is the whole of the wiring, and there is deliberately very little of it —
// every RENDER decision it makes is one the WEB host already makes with the
// same modules:
//
//   gui/host-channel.js         resolve string ids in the payload (issue #949)
//   gui/host-lobby-view.js      payload + previous phase -> view model (#1229)
//   gui/host-lobby-render.js    view model -> the DOM inside #lobby-panel (#1325)
//   gui/host-qr.js              the join panel: the draw, and its one visibility
//                               law (#1329)
//   gui/join-url.js             where a code sends a phone (#1329)
//   gui/host-scenarios.js       catalogue + lock state -> which picker stage
//                               (#1230)
//   gui/host-scenario-render.js that stage -> the DOM inside #scenario-panel
//                               (#1328)
//
// If this file ever grows a render decision of its own, that decision has
// escaped the shared path and belongs back in one of those modules instead.
//
// What it DOES own alone is the send half (issue #1330): the web host has no
// monitors to offer and therefore no monitor row to press, so the click
// listener at the bottom has no counterpart in server.html and no shared module
// to live in.
import './gui/strings-boot.js';
import { t, localiseTree, applyToDom } from './gui/strings.js';
import { localiseHostPayload } from './gui/host-channel.js';
import { hostLobbyViewModel } from './gui/host-lobby-view.js';
import { renderHostLobby, MONITOR_BUTTON_ATTR } from './gui/host-lobby-render.js';
import { applyQrPhase, drawJoinQr, showJoiningOff, toggleQr } from './gui/host-qr.js';
import { joinUrlForCode } from './gui/join-url.js';
import { scenarioCatalogView } from './gui/host-scenarios.js';
import { renderHostScenarios } from './gui/host-scenario-render.js';

// The static `data-i18n` markup — "CREW", "CONNECTED", the awaiting-selection
// badge, the join panel's caption, this surface's QR toggle, the picker's
// "SELECT A WORLD" heading and the AI-launch button — is substituted once here,
// exactly as server.html's own module island does it. Everything data-driven is
// resolved per render by `t` below.
applyToDom(document);

// `hostLobbyViewModel`'s second argument is the phase seen on the previous
// call: it is what makes the Loading -> InProgress edge distinguishable from
// simply being InProgress. The web host keeps it in `_lobbyPrevPhase`; this is
// the same variable under a different roof. Nothing on this surface acts on the
// edges it drives (there is no audio and no loading overlay here), but the view
// model is shared, so it is fed honestly rather than fed a constant.
let prevPhase = '';

const strings = { t, localiseTree };

window.__phoenixHostLobby.render = function (json, revealChrome, layoutJson) {
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
  // The monitor row (issue #1330). It does NOT cross localiseHostPayload: its
  // only free-form text is a display's own OS-reported name, which is never a
  // string id, and its sentences are already `{ id, params }` pairs the
  // renderer resolves. A bad row leaves the row absent and the rest of the
  // lobby rendering — the crew is watching this surface, and a monitor button
  // is not worth blanking it for.
  let layout = null;
  if (layoutJson) {
    try {
      layout = JSON.parse(layoutJson);
    } catch (e) {
      console.warn('[host-lobby] bad monitor row json', e);
    }
  }
  const vm = hostLobbyViewModel(payload, prevPhase, layout);
  prevPhase = payload.phase;
  renderHostLobby(document, vm, t, { revealChrome });
  // The join panel's visibility follows the same law on this surface as on the
  // host page, from the same view model: shown in the lobby, hidden when the
  // mission starts, and LEFT ALONE in play so that a code opened for a late
  // arrival stays open (issue #1329).
  applyQrPhase(document, vm.transitions.qrOverlayAction);
};

// The join panel's own render (issue #1329): what the QR encodes, and the
// presses that have arrived since the last one.
window.__phoenixHostLobby.renderJoin = function (json, qrToggles) {
  for (let i = 0; i < qrToggles; i += 1) toggleQr(document);
  if (!json) return;
  let invite;
  try {
    invite = JSON.parse(json);
  } catch (e) {
    console.warn('[host-lobby] bad join invite json', e);
    return;
  }
  if (invite.kind === 'off') {
    // `--solo`, or a host given no `--rendezvous`. There will never be a code,
    // so the panel says that instead of framing one that cannot work.
    showJoiningOff(document, t('server.join.joining_off'));
    return;
  }
  // The same function the browser host builds its QR from — one join-URL
  // implementation, two hosts. What differs is only the page the client bundle
  // sits beside: a browser host passes its own `location.href`, and this
  // surface is passed the address its host's delivery server is reachable at,
  // because the URL THIS document loaded from is the loopback one no phone can
  // open (native_host::host_lobby::join).
  const url = joinUrlForCode(invite.page_base, invite.full, invite.rendezvous || undefined);
  // `link: false`: this is an embedded view with no tab bar and no second
  // window, so following the QR's own href would navigate the lobby away and
  // leave the viewscreen showing a phone console with no way back.
  drawJoinQr(document, { url, code: invite.code }, window.QRCode, { link: false });
};

// ── Page -> host (issues #1328/#1330) ───────────────────────────────────────
//
// Unlike the renders above, this surface SENDS: a scenario, a hull, the AI
// launch (issue #1328) and a monitor for the viewscreen (issue #1330). All four
// go over the ONE page->host queue the boot script installed, as
// native_host::host_lobby::HostLobbyRecord — four tags in one vocabulary, and
// deliberately not ClientMessages, because this surface holds no session token
// and is not a participant. The host drains that queue in one system and
// dispatches on the tag; a second queue or a second record type would be a
// queue two readers fight over. The host arbitrates a pick
// (src/lobby/scenario_arbiter.rs) and judges a monitor press against the layout
// law, and either answer comes back as the next push.
//
// Everything below sends through here rather than touching
// `phoenixHostLobbyOut` directly, so a queue that is not there yet is one
// console error and not a listener that throws out of an event handler.
function send(record) {
  try {
    window.phoenixHostLobbyOut.send(JSON.stringify(record));
  } catch (e) {
    console.error('[host-lobby] could not send', e);
  }
}

// What the host last told us is locked. Read by `shipStillNeeded` below, and by
// the auto-resolve latch beside it — both of which need the AUTHORITATIVE
// answer rather than "what we asked for", because the arbiter is
// first-valid-wins and a phone can win.
let lockedShip = null;
// The hull the single-hull auto-resolve has already asked for. The browser host
// needs no such latch: its arbiter is in the same page, so the pick lands
// before the next render. Here the answer is a round trip, and every push
// arriving in the meantime would re-send the same request.
let autoAsked = null;

window.__phoenixHostLobby.renderScenario = function (json) {
  let payload;
  try {
    payload = JSON.parse(json);
  } catch (e) {
    console.warn('[host-lobby] bad scenario panel json', e);
    return;
  }
  lockedShip = payload.locked_ship || null;
  if (!lockedShip) autoAsked = null;
  // The SAME call server.html makes, over the same view model: the payload's
  // three fields are `scenarioCatalogView`'s three arguments, which is the whole
  // reason it carries those three and nothing else.
  const vm = scenarioCatalogView(
    payload.scenarios,
    { scenario_id: payload.locked_scenario, template_path: payload.locked_ship },
    payload.locked,
  );
  renderHostScenarios(
    document,
    vm,
    t,
    {
      // A world's `[[available_ships]] label` and a scenario's `label` are
      // authored as string ids (issue #949). `localiseTree` is the same rule
      // server.html's `tData` applies: substitute only what the table holds, so
      // a mod pack's literal prose passes through.
      tData: (value) => (value ? localiseTree(value) : ''),
      selectScenario: (scenarioId) => send({ kind: 'select_scenario', scenario_id: scenarioId }),
      selectShip: (templatePath) => send({ kind: 'select_ship', template_path: templatePath }),
      autoSelectShip: (templatePath) => {
        if (autoAsked === templatePath) return;
        autoAsked = templatePath;
        send({ kind: 'select_ship', template_path: templatePath });
      },
      shipStillNeeded: () => lockedShip === null,
    },
    // This document has no driveWorldLoad() and no return-to-lobby handler, so
    // the payload is the whole of what it knows about whether the picker
    // belongs on screen. server.html passes nothing here and keeps the page
    // lifecycle it always had.
    { ownPanelVisibility: true },
  );
};

// The lobby's AI-launch control (issue #1328). Visible exactly when the shared
// view model says so — `renderHostLobby` sets its display from
// `vm.aiLaunchVisible`, on this surface as on the host page — and pressed, it
// asks the host for the same force start the host page's button asks for. The
// rule that answers is one rule (server::bridge::apply_force_start), not two.
const aiLaunch = document.getElementById('ai-launch-btn');
if (aiLaunch) {
  aiLaunch.addEventListener('click', () => send({ kind: 'force_start' }));
}

// This surface's own QR control. A click, handled here and not sent anywhere:
// the panel's visibility is this document's DOM, and a round trip through the
// host would add a frame of latency to a decision nobody else needs to know.
// A phone's ToggleQrCode reaches the same `toggleQr` from the other direction.
const qrToggle = document.getElementById('host-lobby-qr-toggle');
if (qrToggle) {
  qrToggle.addEventListener('click', () => toggleQr(document));
  // Keyboard parity (issue #1128): the surface is in the pane input router, so
  // it can be reached by a keyboard, and a control a keyboard can focus but not
  // press is worse than one it cannot reach.
  qrToggle.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggleQr(document);
    }
  });
}

// Page -> host: a monitor button press (issue #1330). Delegated off the row's
// container rather than bound per button, because `renderHostLobby` REPLACES
// the buttons on every render — a per-button listener would be rebound sixty
// times a lobby and lost the frame a render happened between the mousedown and
// the click.
//
// What it sends is a request to rearrange this machine's own screens: it
// carries no token, names no participant, and the host judges it against the
// bridge layout law rather than against command admission. It rides the same
// `send` and the same queue as the picks above — one vocabulary, one drain.
//
// The tag is KEBAB where the picks are snake_case. That is not a slip: this
// spelling shipped in #1330 and the Rust side keeps an explicit `serde(rename)`
// for it rather than make a bundle older than the host stop working.
document.addEventListener('click', (ev) => {
  const target = ev.target && ev.target.closest
    ? ev.target.closest(`[${MONITOR_BUTTON_ATTR}]`)
    : null;
  if (!target) return;
  const monitor = target.getAttribute(MONITOR_BUTTON_ATTR);
  if (!monitor) return;
  send({ kind: 'set-viewscreen', monitor });
});

// Anything the host pushed while this island was still loading renders now.
window.__phoenixHostLobby.paint();
window.__phoenixHostLobby.paintJoin();
window.__phoenixHostLobby.paintScenario();
