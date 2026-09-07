/**
 * gui/native-settings.js — the settings cog on the native host's viewscreen
 * surface (issue #1367, PRD #1355).
 *
 * The native host has had no settings of any kind. This gives it one, and it is
 * deliberately NOT a third settings implementation: the shell (find-or-create
 * the cog and the modal, the focus trap, open/close, backdrop dismiss, the tab
 * strip, the section/row primitives) is `gui/settings-overlay-kit.js`, exactly
 * as the host page's cog (`gui/server-settings.js`, issue #939) and the phone's
 * (`gui/settings-panel.js`, issue #940) use it, and the tab list is the shared
 * `gui/settings-tabs.js`. What is here is only this surface's tab BODIES —
 * which is the one thing the kit's own doc says must stay per-page, because the
 * three surfaces reach what they control down genuinely different paths.
 *
 * ## The controls are a TABLE, not a switch
 *
 * The same shape `gui/host-landing-view.js` gives the menu, and for the same
 * reason: this surface will grow settings as the native host grows things to
 * set, and "add a control" must mean adding a row rather than editing a builder.
 * A row says which tab it lives on, which section it sits under, what it is
 * called and what verb it sends. Nothing below reads a control id by name.
 *
 * ## A tab with nothing on it is not offered
 *
 * The shared tab list is the four the host page shows, and three of them are
 * empty here TODAY — the native process has no master volume, no wasm debug
 * bindings and no host-local action registry. Rendering them as blank bodies
 * would be the settings menu claiming settings this host does not have, which
 * is the same failure as a control with nothing behind it. So the tab strip is
 * the shared list FILTERED to the tabs the table puts something on: an empty
 * tab disappears, and the slice that gives the native host a volume brings its
 * tab back by adding a row. `visibleTabs` still decides the order and still
 * decides what a demo build hides, so this never becomes a second tab list.
 *
 * ## Everything it cannot do itself arrives as a hook
 *
 * Two verbs today and they cost different things, which is why they are hooks
 * and not calls: the join QR is this document's own DOM (`gui/host-qr.js`'s
 * toggle, reached the same way this surface's corner control reaches it), and
 * the window mode is the host PROCESS's and crosses the page->host queue. This
 * module knows neither. It hands the row's `action` to `hooks.run` and the
 * caller with something behind it decides what that means — the same
 * arrangement `renderHostLanding`'s `confirm` and `installPack` hooks make.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

import { t as defaultT } from './strings.js';
import { visibleTabs } from './settings-tabs.js';
import {
  mountOverlayShell,
  renderTabBar,
  makeSectionBuilders,
  makeRowBuilder,
} from './settings-overlay-kit.js';

/** The cog's element id. Exported so a test and the CSS agree on one name. */
export const NATIVE_SETTINGS_BUTTON_ID = 'native-settings-btn';

/** The modal's element id. */
export const NATIVE_SETTINGS_OVERLAY_ID = 'native-settings-overlay';

/**
 * Everything the native surface can actually set, one row per control.
 *
 * | field | what it decides |
 * |---|---|
 * | `id` | the machine key the row is known by, and the `data-control` a test presses |
 * | `tab` | which shared tab it appears on — a tab no row names is not offered |
 * | `sectionId` | the string id of the heading it sits under; rows sharing one share a section |
 * | `labelId` | the string id of the control's own name |
 * | `hintId` | the string id of the line under the section, or absent |
 * | `action` | the verb handed to the caller's `run` hook when it is pressed |
 *
 * Both rows are on **Gameplay** because both are decisions about the session in
 * front of the operator rather than about this machine's audio or bindings,
 * which is where the host page puts the same QR control.
 */
export const NATIVE_SETTINGS_CONTROLS = [
  {
    // The one control this surface already had, as a bespoke `<div
    // role="button">` the document supplies
    // (`native_host::host_lobby::document`'s `QR_TOGGLE_MARKUP`). That control
    // exists BECAUSE this window had no cog; it now has one, and the cog offers
    // the same decision under the same string the host page's Gameplay tab uses
    // for it. Two triggers, one implementation — `gui/host-qr.js`'s `toggleQr`
    // — which is the shape three callers already reach that toggle by. The
    // bespoke control stays: it is one press from the screen the crew are
    // looking at, and a join code is what a room needs fastest.
    id: 'qr',
    tab: 'gameplay',
    sectionId: 'settings.qr_code',
    labelId: 'settings.toggle_qr',
    action: 'toggle_qr',
  },
  {
    // The window mode (issue #1367). The same verb the landing's corner control
    // sends, named once here and once on the row that owns the corner — see
    // `host_lobby_link.js`, which forwards both to one `send`.
    id: 'fullscreen',
    tab: 'gameplay',
    sectionId: 'settings.display',
    labelId: 'settings.toggle_fullscreen',
    hintId: 'settings.display_hint',
    action: 'toggle_fullscreen',
  },
];

/**
 * What the overlay offers, as a pure decision.
 *
 * No DOM, no `window`, no string resolution — ids in, ids out — so "which tabs
 * does the native host show, and what is on them" is a unit test rather than a
 * claim about a builder. The exact sibling of `landingViewModel`.
 *
 * @param {{demo?: boolean, activeTab?: string|null, controls?: Array<object>}} [input]
 *   `activeTab` is the caller's own memory of which tab is selected, held there
 *   rather than here for the reason `landingViewModel` takes `openEntryId`: the
 *   surface owns its lifecycle. A tab that is not offered — because a demo
 *   build hid it, or because nothing is on it — reads as the first one that is.
 * @returns {{
 *   tabs: Array<{id: string, labelId: string}>,
 *   activeTab: string|null,
 *   sections: Array<{id: string, headingId: string, hintId: string|null,
 *                    controls: Array<{id: string, labelId: string, action: string}>}>,
 * }}
 *   `sections` is the ACTIVE tab's, in table order, and empty when nothing is
 *   offered at all.
 */
export function nativeSettingsView(input) {
  const opts = input || {};
  const rows = Array.isArray(opts.controls) ? opts.controls : NATIVE_SETTINGS_CONTROLS;
  // The shared list decides the order and decides what a demo build hides; this
  // only drops the ones nothing on this surface can answer.
  const tabs = visibleTabs(!!opts.demo)
    .filter(function (tab) {
      return rows.some(function (row) { return row && row.tab === tab.id; });
    })
    .map(function (tab) {
      return { id: tab.id, labelId: tab.labelId };
    });
  const active = tabs.some(function (tab) { return tab.id === opts.activeTab; })
    ? opts.activeTab
    : (tabs.length ? tabs[0].id : null);

  const sections = [];
  rows.forEach(function (row) {
    if (!row || !row.id || row.tab !== active) return;
    let section = sections.find(function (s) { return s.id === row.sectionId; });
    if (!section) {
      section = {
        id: row.sectionId,
        headingId: row.sectionId,
        hintId: null,
        controls: [],
      };
      sections.push(section);
    }
    // The first row that names a hint gives the section one. A hint belongs to
    // the section rather than to the control because that is where the kit's
    // primitives put it, and a second row wanting different words wants a
    // second section.
    if (!section.hintId && row.hintId) section.hintId = row.hintId;
    section.controls.push({ id: row.id, labelId: row.labelId, action: row.action });
  });

  return { tabs: tabs, activeTab: active, sections: sections };
}

/**
 * Mount the cog and its modal onto `doc`.
 *
 * @param {Document} doc the document to mount into. FIRST, as every renderer in
 *   this fleet takes it: one implementation, several documents.
 * @param {{run?: (action: string) => void}} [hooks] `run` is handed the pressed
 *   row's `action` verb. Absent, the controls render and do nothing — which is
 *   what a surface with nothing behind them would be, and is why the table says
 *   what a row sends rather than doing it.
 * @param {{t?: (id: string, params?: object) => string, demo?: boolean}} [opts]
 *   `t` is injected so a surface that reaches the String Table its own way can
 *   say so. The kit's own chrome (the cog's accessible name, the tab labels)
 *   resolves through the kit's import, which is the kit's contract and not this
 *   module's to change here.
 * @returns {{open: () => void, close: () => void, isOpen: () => boolean,
 *            selectTab: (id: string) => void, btn: Element, overlay: Element}}
 */
export function mountNativeSettings(doc, hooks, opts) {
  const h = hooks || {};
  const o = opts || {};
  const t = o.t || defaultT;
  const demo = !!o.demo;

  const shell = mountOverlayShell(doc, {
    buttonId: NATIVE_SETTINGS_BUTTON_ID,
    overlayId: NATIVE_SETTINGS_OVERLAY_ID,
    // Two classes, and the kit hands the string straight to `className`: the
    // LOOK is `gui/host-landing.css`'s `.landing-icon-btn` — the same chamfered
    // square the fullscreen control in the opposite corner wears, because the
    // design draws them as a pair — and `.native-settings-btn` is only where
    // this sheet puts it. One chrome vocabulary, not a second one for the cog.
    buttonClass: 'landing-icon-btn native-settings-btn',
    overlayClass: 'native-settings-overlay',
  });

  const { section, hint } = makeSectionBuilders(doc, {
    sectionClass: 'native-settings-section',
    headingClass: 'native-settings-heading',
    hintClass: 'native-settings-hint',
  });
  const row = makeRowBuilder(doc, 'native-settings-row');

  /** This surface's memory of which tab is selected. See `nativeSettingsView`. */
  let activeTab = null;

  function buildPanel() {
    const vm = nativeSettingsView({ demo: demo, activeTab: activeTab, controls: o.controls });
    activeTab = vm.activeTab;

    // Rebuilt whole on every open, like both siblings: a panel holding the last
    // render's rows is a panel that shows them for one frame the next time it
    // opens.
    shell.overlay.innerHTML = '';

    const popup = doc.createElement('div');
    popup.className = 'native-settings-popup';

    const title = doc.createElement('div');
    title.className = 'native-settings-title';
    title.textContent = t('settings.title');
    popup.appendChild(title);

    const tabBar = doc.createElement('div');
    tabBar.className = 'native-settings-tabs';
    renderTabBar(doc, tabBar, vm.tabs, vm.activeTab, 'native-settings-tab', function (id) {
      activeTab = id;
      buildPanel();
    });
    popup.appendChild(tabBar);

    const body = doc.createElement('div');
    body.className = 'native-settings-body';
    vm.sections.forEach(function (spec) {
      const el = section(spec.headingId);
      if (spec.hintId) el.appendChild(hint(spec.hintId));
      const controls = row();
      spec.controls.forEach(function (control) {
        const btn = doc.createElement('button');
        btn.type = 'button';
        btn.className = 'native-settings-control';
        btn.setAttribute('data-control', control.id);
        btn.textContent = t(control.labelId);
        btn.addEventListener('click', function (e) {
          if (e && typeof e.preventDefault === 'function') e.preventDefault();
          if (h.run) h.run(control.action);
        });
        controls.appendChild(btn);
      });
      el.appendChild(controls);
      body.appendChild(el);
    });
    popup.appendChild(body);
    shell.overlay.appendChild(popup);
  }

  shell.buildContent = buildPanel;

  return {
    btn: shell.btn,
    overlay: shell.overlay,
    open: shell.open,
    close: shell.close,
    isOpen: shell.isOpen,
    selectTab: function (id) {
      activeTab = id;
      if (shell.isOpen()) buildPanel();
    },
  };
}

// Expose for a classic-script consumer, the same self-registering pattern
// window.hostLanding and window.hostLandingRender use. Nothing loads this as a
// classic script today — `host_lobby_link.js` is a module and imports it — but
// a module that is only reachable one way is a module the next surface has to
// change to reuse.
if (typeof window !== 'undefined') {
  window.nativeSettings = {
    NATIVE_SETTINGS_CONTROLS,
    NATIVE_SETTINGS_BUTTON_ID,
    NATIVE_SETTINGS_OVERLAY_ID,
    nativeSettingsView,
    mountNativeSettings,
  };
}
