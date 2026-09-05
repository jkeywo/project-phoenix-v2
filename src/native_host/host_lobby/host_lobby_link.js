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
//   gui/host-landing-view.js    which routes the menu offers, and which one a
//                               press opens or closes (#1360)
//   gui/host-landing-render.js  that decision -> the DOM inside #landing-panel,
//                               the mod-pack shelf included (#1360, #1366)
//   gui/native-settings.js      this surface's settings cog and modal, built
//                               from the SHARED overlay kit and the SHARED tab
//                               list (#1367) — the host page's cog and the
//                               phone's are the other two consumers of both
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
import {
  renderHostLobby,
  MONITOR_BUTTON_ATTR,
  STATION_BUTTON_ATTR,
  STATION_SCREEN_ATTR,
} from './gui/host-lobby-render.js';
import { applyQrPhase, drawJoinQr, showJoiningOff, toggleQr } from './gui/host-qr.js';
import { joinUrlForCode } from './gui/join-url.js';
import { scenarioCatalogView } from './gui/host-scenarios.js';
import { renderHostScenarios } from './gui/host-scenario-render.js';
import { landingEntries, landingViewModel, nextOpenEntry } from './gui/host-landing-view.js';
import { renderHostLanding } from './gui/host-landing-render.js';
import { mountNativeSettings } from './gui/native-settings.js';

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

// ── Page -> host (issues #1328/#1330/#1331/#1361/#1365/#1366) ───────────────
//
// Unlike the renders above, this surface SENDS: a scenario, a hull, the AI
// launch (issue #1328), a monitor for the viewscreen (issue #1330), a screen
// for a station's console — or none, closing it (issue #1331) — a route
// opened or closed on the landing menu (issue #1361), the confirmed Exit
// to Desktop (issue #1365) and a mod pack chosen off the shelf (issue #1366).
// All ten go over
// the ONE page->host queue the boot script installed, as
// native_host::host_lobby::HostLobbyRecord — ten tags in one vocabulary, and
// deliberately not ClientMessages, because this surface holds no session token
// and is not a participant. The host drains that queue in one system and
// dispatches on the tag; a second queue or a second record type would be a
// queue two readers fight over. The host arbitrates a pick
// (src/lobby/scenario_arbiter.rs) and judges a layout press against the layout
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

// ── The landing screen (issue #1361) ────────────────────────────────────────
//
// `_landingOpenEntry` is this document's own memory of which route is open, and
// it lives here for the reason server.html's does: it is THIS surface's
// lifecycle, and a module-level flag two documents shared would be a second
// authority the moment either forgot to update it. WHAT an open entry means, and
// whether a press opens anything at all, is `nextOpenEntry`'s — the same pure
// function the host page calls, over the same shipped table. Nothing about the
// menu is decided in this file; `platforms: ['web']` on the Connect-to-Host row
// is why that entry is not on this surface, and it is said in the table.
//
// `nextOpenEntry` is asked over `landingEntries('native')` and not over the raw
// table, for the reason that function documents: a row this surface offers but
// cannot open yet — Load Game, until #1363's AC5 lands — comes back with its
// stage taken away, and a click judged against the full table would leave this
// file remembering an entry as open that `drawLanding` renders as closed.
let landingOpenEntry = null;
// The host's last word about the landing: which build this is, and whether a
// World has taken the front door away. Held so a re-render driven by a click
// carries the same facts the last push did.
let landingState = { build: 'dev', dismissed: false };
// The host's mod-pack shelf (issue #1366), or null on a host started without
// `--mod-pack-dir` — which is most of them, and is exactly how the landing's
// Load-mod-pack row stays inert here. `landingProvides` is what this surface
// tells `landingViewModel` it can ANSWER, and it is derived from the shelf
// rather than declared: a capability list that said `packs` on a host that
// never pushed one would be this file claiming something the process cannot do.
let landingPacks = null;
// Which archive the operator has highlighted. This surface's own memory, for the
// reason `landingOpenEntry` is: the host hears about a choice when it is asked
// to install one, and holding it there would make every highlight a round trip.
let landingPackChoice = null;

function landingProvides() {
  return landingPacks ? ['packs'] : [];
}

function drawLanding() {
  renderHostLanding(
    document,
    landingViewModel({
      openEntryId: landingOpenEntry,
      // A fact about this document, not a decision about it: this file is
      // loaded by exactly one surface, and that surface is the native one.
      platform: 'native',
      build: landingState.build,
      dismissed: landingState.dismissed,
      provides: landingProvides(),
      packs: landingPacks,
      chosenPack: landingPackChoice,
    }),
    t,
    {
      pick: (entryId) => {
        // Both lists, because both are true of this surface at once: the
        // menu `landingEntries('native')` gives is the one `drawLanding`
        // renders (so a row this host cannot open yet is judged closed here
        // too), and `landingProvides()` is what this particular RUN can
        // answer (so the mod-pack row is inert without a scanned folder).
        const next = nextOpenEntry(
          landingOpenEntry, entryId, landingEntries('native'), landingProvides(),
        );
        // An entry whose slice has not landed returns the open entry unchanged,
        // and the view model is a pure function of that memory - so re-rendering
        // would rebuild the whole menu to produce byte-identical DOM, at the
        // cost of the node a keyboard operator is standing on. Same reason
        // server.html skips it.
        if (next === landingOpenEntry) return;
        landingOpenEntry = next;
        send(next ? { kind: 'landing_open', entry: next } : { kind: 'landing_close' });
        drawLanding();
      },
      // A confirmation's own control (issue #1365). `action` is the verb the
      // OPEN ROW declared — the quit verb today — and it is sent as the record
      // `kind` rather than translated through a table here, so a confirming row
      // names its verb once, in the one place a row is declared. The host
      // dispatches on the tag like every other record and warns about one it
      // does not speak, which is what an unknown verb should look like on a
      // surface whose page can be older than the binary serving it.
      //
      // Nothing is redrawn afterwards, and nothing should be: what answers the
      // quit verb is an application exit, so the next thing this window
      // does is go away. A hopeful re-render would be this document claiming to
      // know that the host agreed.
      confirm: (action) => send({ kind: action }),
      // The mod-pack shelf's two (issue #1366), and they are two because they
      // cost different things. Highlighting a row is this document's own memory
      // and is answered by a repaint; installing reads an archive off a disk,
      // runs the whole validation and changes the catalogue every phone in the
      // room is looking at, so it is a record the host answers.
      pickPack: (file) => {
        if (landingPackChoice === file) return;
        landingPackChoice = file;
        drawLanding();
      },
      // `action` is the verb the OPEN ROW declared, forwarded as the record's
      // `kind` rather than translated through a table here — the same
      // arrangement `confirm` above makes, and the reason a row names its verb
      // once in the one place a row is declared.
      //
      // Nothing is redrawn afterwards, and nothing should be: what answers this
      // is a push carrying the host's own account of what happened, and a
      // hopeful repaint here would be this document claiming to know the answer
      // before it arrives.
      installPack: (action, file) => send({ kind: action, pack: file }),
      // The corner fullscreen control (issue #1367). #1361 handed over no hook
      // and the document stripped the control with it, because a browser host's
      // forwards to `gui/page-chrome.js`'s one `initFullscreen` — which asks a
      // BROWSER to fill a screen, and this window has no browser chrome. What
      // fullscreen means here is the primary window's mode, which belongs to
      // the host process, so the press is a record like every other thing this
      // surface cannot do itself.
      //
      // Nothing is redrawn afterwards, and nothing should be: what answers it
      // is `fullscreen::apply_window_mode_toggle` moving a window, and this
      // document cannot see a window mode at all — there is no
      // `document.fullscreenElement` on an embedded view. A repaint here would
      // be the page claiming to know an answer only the host has.
      toggleFullscreen: () => send({ kind: 'toggle_fullscreen' }),
    },
    // This document has no page lifecycle: its host is the only thing that
    // knows a World has been committed, which is what `dismissed` carries and
    // what `ownPanelVisibility` lets the shared renderer act on. server.html
    // passes neither and keeps hideLanding()/showLandingAtPicker().
    { ownPanelVisibility: true },
  );
}

// Host -> page: the landing's two facts. Every render goes through
// `drawLanding` so a push and a click cannot draw two different landings.
window.__phoenixHostLobby.renderLanding = function (json) {
  let payload;
  try {
    payload = JSON.parse(json);
  } catch (e) {
    console.warn('[host-lobby] bad landing json', e);
    return;
  }
  landingState = {
    build: payload.build || 'dev',
    dismissed: !!payload.dismissed,
  };
  // A dismissed landing drops the open route with it, so a landing brought back
  // later (a Game Over returning this host to selection) opens on its front
  // door rather than on a stage nobody asked for.
  if (landingState.dismissed) landingOpenEntry = null;
  drawLanding();
};

// Host -> page: the mod-pack shelf (issue #1366). Arriving at all is what tells
// this surface it can answer the Load-mod-pack row, so a host started without
// `--mod-pack-dir` — which never pushes — leaves that row inert with no check
// for the flag anywhere on this side of the bridge.
window.__phoenixHostLobby.renderPacks = function (json) {
  let payload;
  try {
    payload = JSON.parse(json);
  } catch (e) {
    console.warn('[host-lobby] bad mod-pack shelf json', e);
    return;
  }
  landingPacks = payload;
  // A highlight the host is no longer offering is dropped rather than carried:
  // the shelf is rescanned on every attempt, so the archive an operator chose
  // can have left the folder — and an install of a pack that is not on the
  // shelf is a refusal, which is a worse way to find that out than the row
  // simply no longer being there.
  if (landingPackChoice
    && !(payload.offered || []).some((p) => p.file === landingPackChoice)) {
    landingPackChoice = null;
  }
  drawLanding();
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

// ── The settings cog (issue #1367) ─────────────────────────────────────────
//
// The native host had no settings of any kind, and this is the EXISTING one
// rather than a third: `gui/native-settings.js` builds the cog and the modal
// from `gui/settings-overlay-kit.js` and takes its tabs from
// `gui/settings-tabs.js`, which are the same shell and the same list the host
// page's cog (#939) and the phone's (#940) use. What is per-surface is the tab
// BODIES, which the kit's own doc says must stay per-surface — the three
// surfaces reach what they control down genuinely different paths, and this one
// reaches everything through the two hooks below.
//
// Mounted unconditionally, and it is not a control with nothing behind it: both
// verbs on its table are answered here, one in this document and one by the
// host. A tab with no control on this surface is not offered at all, so the
// panel can never show an empty Audio page — see `nativeSettingsView`.

// The verbs THIS DOCUMENT answers itself, one row each. Everything not in here
// is a record the host answers, which is the default rather than a case: the
// QR panel's visibility is this document's own DOM and a round trip would add a
// frame of latency to a decision nobody else needs to know, while the window
// mode is the host process's and cannot be reached from a page at all. A
// settings control that the surface can serve locally is a row here; one the
// host serves is a row in `NATIVE_SETTINGS_CONTROLS` and nothing here at all.
const LOCAL_SETTINGS_VERBS = {
  toggle_qr: () => toggleQr(document),
};

mountNativeSettings(document, {
  // The row's verb, forwarded — never a name this file decides.
  run: (action) => {
    const local = LOCAL_SETTINGS_VERBS[action];
    if (local) local();
    else send({ kind: action });
  },
}, { t });

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

// Page -> host: a layout button press — the viewscreen's monitor row
// (issue #1330) and a station's screen row (issue #1331). Delegated off the
// document rather than bound per button, because `renderHostLobby` REPLACES
// every one of them on every render — a per-button listener would be rebound
// sixty times a lobby and lost the frame a render happened between the mousedown
// and the click.
//
// What it sends is a request to rearrange this machine's own screens: it
// carries no token, names no participant, and the host judges it against the
// bridge layout law rather than against command admission. Which station's
// console a screen shows is a LAYOUT question — never a question of who may sit
// at it, which stays the ordinary claim flow a phone goes through. All three
// verbs ride the same `send` and the same queue as the picks above — one
// vocabulary, one drain.
//
// The tags are KEBAB where the picks are snake_case. That is not a slip:
// `set-viewscreen` shipped that way in #1330, its two station siblings were
// written to match it, and the Rust side keeps an explicit `serde(rename)` for
// each rather than make a bundle older than the host stop working.
//
// The station row is tested FIRST. Its buttons deliberately do not carry
// `data-monitor` (see `host-lobby-render.js`), so the order is belt to braces
// rather than the thing that keeps them apart — but an ordering that reads
// "the more specific control wins" is the one that survives somebody adding a
// third row.
document.addEventListener('click', (ev) => {
  const closest = (attr) => (ev.target && ev.target.closest
    ? ev.target.closest(`[${attr}]`)
    : null);

  const station = closest(STATION_BUTTON_ATTR);
  if (station) {
    const id = station.getAttribute(STATION_BUTTON_ATTR);
    if (!id) return;
    // Empty is the off state — a real value the host acts on, not a hole.
    const monitor = station.getAttribute(STATION_SCREEN_ATTR) || '';
    send(
      monitor
        ? { kind: 'assign-station', station: id, monitor }
        : { kind: 'unassign-station', station: id },
    );
    return;
  }

  const target = closest(MONITOR_BUTTON_ATTR);
  if (!target) return;
  const monitor = target.getAttribute(MONITOR_BUTTON_ATTR);
  if (!monitor) return;
  send({ kind: 'set-viewscreen', monitor });
});

// Anything the host pushed while this island was still loading renders now.
window.__phoenixHostLobby.paint();
window.__phoenixHostLobby.paintJoin();
window.__phoenixHostLobby.paintScenario();
window.__phoenixHostLobby.paintLanding();
window.__phoenixHostLobby.paintPacks();
