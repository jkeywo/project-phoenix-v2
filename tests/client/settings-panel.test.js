// Issue #940 — the phone client's settings cog.
//
// The panel is the mirror of the host page's (issue #939): same shared tabs,
// same tab gated in a demo build. Everything it decides is a pure exported
// function, so the interesting cases — which tab survives the build, what a
// debug button shows before the server has ever reported, what actually goes on
// the wire — are driven here without a browser.
//
// The DOM stub is the same minimal one help-panel.test.js uses; the mount tests
// below drive the real module against it.

import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  mountSettings,
  buildSettingsState,
  createMasterVolume,
  clampVolume,
  debugFlagMessage,
  pauseMessage,
  godModeMessage,
  CLIENT_DEBUG_FLAGS,
  PAUSE_CONTROL_ID,
  GOD_MODE_SYSTEM_ID,
  isSemanticModifierEvent,
  semanticModifierCode,
  operatorProfileStatusView,
} from '../../gui/settings-panel.js';
import { setBuildFlags, isDemoBuild } from '../../gui/build-flags.js';
import { TEXT_SCALE_MAX } from '../../gui/accessibility-profile.js';
import {
  TABS,
  CLIENT_ACCESSIBILITY_TABS,
  CLIENT_DOCUMENTATION_TABS,
} from '../../gui/settings-tabs.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { createSemanticActionRegistry } from '../../gui/semantic-action-registry.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import {
  CAPTAIN_RED_ALERT_ACTION_ID,
  CAPTAIN_VIEW_ACTION_ID,
  createCaptainActionRegistry,
} from '../../gui/stations/captain-actions.js';

const repoFile = (rel) =>
  fs.readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), '../..', rel),
    'utf-8',
  );

/** client.html itself — the cog's stacking and the build tag are page facts. */
const CLIENT_HTML = repoFile('client.html');
/** The consoles' shared stylesheet, which reserves the cog's corner for it. */
const CONSOLE_CSS = repoFile('gui/console.css');
/** The shared token vocabulary — the shell's own root size lives here. */
const TOKENS_CSS = repoFile('gui/tokens.css');
/** Shared page chrome (issue #1227) — client.html mounts it rather than
 *  inlining the fullscreen icon-sync logic itself. */
const PAGE_CHROME_JS = repoFile('gui/page-chrome.js');

// ── Minimal DOM stub (same pattern as help-panel.test.js) ───────────────────

function makeEl(doc, tag) {
  const listeners = {};
  const el = {
    ownerDocument: doc,
    tagName: String(tag).toUpperCase(),
    children: [],
    attributes: {},
    classList: new Set(),
    _id: '',
    hidden: false,
    textContent: '',
    type: '',
    title: '',
    value: '',
    min: '',
    max: '',
    step: '',
    disabled: false,
    get id() { return this._id; },
    set id(v) { this._id = v; if (v) doc._byId[v] = this; },
    set innerHTML(_v) { this.children = []; },
    setAttribute(k, v) { this.attributes[k] = String(v); },
    getAttribute(k) { return this.attributes[k]; },
    hasAttribute(k) { return k in this.attributes; },
    appendChild(child) { this.children.push(child); child.parentNode = this; return child; },
    addEventListener(type, fn) {
      (listeners[type] = listeners[type] || []).push(fn);
    },
    dispatch(type, ev) {
      (listeners[type] || []).forEach((fn) => fn.call(this, ev || { preventDefault() {}, stopPropagation() {} }));
    },
    click() { this.dispatch('click'); },
    focus() { doc.activeElement = this; this.dispatch('focus'); },
    blur() { if (doc.activeElement === this) doc.activeElement = null; this.dispatch('blur'); },
    querySelector(selector) {
      const match = String(selector).match(/^\[data-control="([^"]+)"\]$/);
      if (!match) return null;
      const wanted = match[1];
      const visit = (node) => {
        for (const child of node.children || []) {
          if (child.getAttribute && child.getAttribute('data-control') === wanted) return child;
          const nested = visit(child);
          if (nested) return nested;
        }
        return null;
      };
      return visit(this);
    },
    querySelectorAll() { return []; },
    closest() { return null; },
    getElementsByClassName() { return []; },
    insertBefore() {},
    get rootNode() { return this; },
    contains() { return false; },
    valueOf() { return this; },
  };
  el.classList.add = (c) => Set.prototype.add.call(el.classList, c);
  el.classList.remove = (c) => Set.prototype.delete.call(el.classList, c);
  el.classList.contains = (c) => Set.prototype.has.call(el.classList, c);
  Object.defineProperty(el, 'className', {
    get() { return Array.from(el.classList).join(' '); },
    set(v) { el.classList.clear(); String(v).split(/\s+/).filter(Boolean).forEach((c) => el.classList.add(c)); },
  });
  return el;
}

function makeDoc() {
  const doc = {
    _byId: {},
    _query: {},
    _listeners: {},
    readyState: 'complete',
    createElement(tag) { return makeEl(this, tag); },
    getElementById(id) { return this._byId[id] || null; },
    querySelector(sel) { return this._query[sel] || null; },
    addEventListener(type, fn) {
      (this._listeners[type] = this._listeners[type] || []).push(fn);
    },
    removeEventListener(type, fn) {
      this._listeners[type] = (this._listeners[type] || []).filter((entry) => entry !== fn);
    },
    dispatch(type, event) {
      for (const fn of [...(this._listeners[type] || [])]) fn(event);
    },
  };
  doc.body = makeEl(doc, 'body');
  doc.documentElement = makeEl(doc, 'html');
  return doc;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const findOverlay = (doc) => doc.getElementById('settings-overlay');
const findBtn = (doc) => doc.getElementById('settings-btn');
const popupOf = (doc) => findOverlay(doc).children[0];
const tabBarOf = (doc) => popupOf(doc).children.find((c) => c.className === 'settings-tabs');
const bodyOf = (doc) => popupOf(doc).children.find((c) => c.className === 'settings-body');

/** Every button in the open panel's body, flattened one level deep. */
function bodyButtons(doc) {
  const out = [];
  for (const section of bodyOf(doc).children) {
    for (const child of section.children) {
      if (child.tagName === 'BUTTON') out.push(child);
      else for (const grand of child.children || []) out.push(grand);
    }
  }
  return out;
}

function mount(doc, opts = {}) {
  return mountSettings({
    doc,
    send() {},
    getState: () => ({ stations: [], stationRatings: {} }),
    myToken: 'tok1',
    isDemo: () => false,
    ...opts,
  });
}

// ── Shell ────────────────────────────────────────────────────────────────────

describe('mountSettings — cog and overlay', () => {
  let doc;
  beforeEach(() => { doc = makeDoc(); });

  it('creates a gear button and a hidden overlay', () => {
    mount(doc);
    expect(findBtn(doc)).not.toBeNull();
    expect(findBtn(doc).textContent).toBe('⚙');
    expect(findBtn(doc).getAttribute('aria-label')).toBe(t('settings.title'));
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(true);
    expect(overlay.getAttribute('aria-hidden')).toBe('true');
  });

  it('defaults to no-ops when no document is available', () => {
    const inst = mountSettings({ doc: null });
    expect(typeof inst.open).toBe('function');
    expect(typeof inst.close).toBe('function');
    expect(typeof inst.rebuildContent).toBe('function');
  });

  it('open() reveals the overlay, close() hides it', () => {
    const inst = mount(doc);
    inst.open();
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(false);
    expect(overlay.getAttribute('aria-hidden')).toBe('false');
    expect(overlay.classList.contains('open')).toBe(true);
    inst.close();
    expect(overlay.hidden).toBe(true);
    expect(overlay.classList.contains('open')).toBe(false);
  });

  it('clicking the gear toggles the overlay', () => {
    mount(doc);
    const overlay = findOverlay(doc);
    findBtn(doc).click();
    expect(overlay.hidden).toBe(false);
    findBtn(doc).click();
    expect(overlay.hidden).toBe(true);
  });

  it('opens on the client controls and documentation tabs, Debug last among the settings tabs', () => {
    const inst = mount(doc);
    inst.open();
    const labels = tabBarOf(doc).children.map((c) => c.getAttribute('data-tab'));
    expect(labels).toEqual(['audio', 'gameplay', 'controls', 'debug', 'accessibility', 'station-help', 'ship-manual']);
    expect(tabBarOf(doc).children[0].classList.contains('active')).toBe(true);
  });

  it('clicking a tab switches the body without closing the panel', () => {
    const inst = mount(doc);
    inst.open();
    tabBarOf(doc).children.find((c) => c.getAttribute('data-tab') === 'audio').click();
    expect(findOverlay(doc).hidden).toBe(false);
    const active = tabBarOf(doc).children.find((c) => c.classList.contains('active'));
    expect(active.getAttribute('data-tab')).toBe('audio');
  });
});

// ── One cog, moved (issue #1372) ─────────────────────────────────────────────
//
// The bar became the page's only header, so the cog it leads and the `?` that
// closes it are the SAME nodes the lobby uses, re-parented — not a second pair
// hidden behind a media query. Two copies would drift: the focus trap, the
// aria-expanded state and the open/close toggle all live on one button.

describe('the cog moves between the bar and the page body', () => {
  let doc;
  beforeEach(() => { doc = makeDoc(); });

  it('is born under a caller-supplied container when one is given', () => {
    const bar = makeEl(doc, 'nav');
    mount(doc, { buttonContainer: bar });
    expect(findBtn(doc).parentNode).toBe(bar);
  });

  it('defaults to the page body, which is where the lobby wants it', () => {
    mount(doc);
    expect(findBtn(doc).parentNode).toBe(doc.body);
  });

  it('re-parents the one node rather than creating a second', () => {
    const bar = makeEl(doc, 'nav');
    const inst = mount(doc);
    const cog = findBtn(doc);

    inst.setButtonContainer(bar);
    expect(cog.parentNode).toBe(bar);
    expect(findBtn(doc)).toBe(cog);

    inst.setButtonContainer(null);
    expect(cog.parentNode).toBe(doc.body);
    expect(findBtn(doc)).toBe(cog);
  });

  it('still toggles its overlay after the move', () => {
    const bar = makeEl(doc, 'nav');
    const inst = mount(doc);
    inst.setButtonContainer(bar);
    findBtn(doc).click();
    expect(findOverlay(doc).hidden).toBe(false);
    findBtn(doc).click();
    expect(findOverlay(doc).hidden).toBe(true);
  });

  it('opens Settings straight onto one tab for the help button', () => {
    const inst = mount(doc);
    expect(inst.isOpen()).toBe(false);
    inst.openTab('station-help');
    expect(inst.isOpen()).toBe(true);
    expect(
      tabBarOf(doc).children.find((c) => c.classList.contains('active'))
        .getAttribute('data-tab'),
    ).toBe('station-help');

    // Already open on another tab: switch, do not close.
    inst.selectTab('audio');
    inst.openTab('station-help');
    expect(inst.isOpen()).toBe(true);
    expect(
      tabBarOf(doc).children.find((c) => c.classList.contains('active'))
        .getAttribute('data-tab'),
    ).toBe('station-help');
  });
});

function allText(el) {
  let text = el.textContent ? [el.textContent] : [];
  for (const child of el.children || []) text = text.concat(allText(child));
  return text;
}

describe('documentation tabs', () => {
  it('renders help only for the station held by this client', () => {
    const doc = makeDoc();
    const inst = mount(doc, {
      getState: () => ({
        stations: [{ id: 'helm', holder_token: 'tok1', ratings: ['Std'] }],
        stationRatings: {},
      }),
    });
    inst.open();
    inst.selectTab('station-help');
    const text = allText(bodyOf(doc)).join('\n');
    expect(text).toContain(t('station.helm.name'));
    expect(text).toContain(t('help.helm.0.heading'));
    expect(text).not.toContain(t('help.repair.0.heading'));
  });

  it('shows the localized unavailable state outside a held station', () => {
    const doc = makeDoc();
    const inst = mount(doc);
    inst.open();
    inst.selectTab('station-help');
    expect(allText(bodyOf(doc))).toContain(t('settings.station_help.unavailable'));
  });

  it('renders the replicated ship manual within Settings without a book trigger', () => {
    const doc = makeDoc();
    const inst = mount(doc, {
      getManual: () => ({ stations: [{ station_id: 'helm', overview: 'Fly the ship.', sections: [] }] }),
    });
    inst.open();
    inst.selectTab('ship-manual');
    expect(allText(bodyOf(doc))).toContain('Fly the ship.');
    expect(doc.getElementById('manual-btn')).toBeNull();
    expect(doc.getElementById('manual-overlay')).toBeNull();
  });

  // ── The reader's place is not a settings fact (PRD #1023's defect list) ──
  //
  // The panel keeps one tab slot, and the Ship Manual has a second tab strip
  // inside it. That inner selection used to live only in a closure over the
  // DOM the panel throws away on every repaint, so any settings-driven rebuild
  // silently sent the reader back to the first station — including a
  // `DebugState` push from the host, which is nothing to do with them.

  const TWO_STATION_MANUAL = {
    stations: [
      { station_id: 'helm', overview: 'Fly the ship.', sections: [] },
      { station_id: 'repair', overview: 'Patch the ship.', sections: [] },
    ],
  };

  /** Click the manual's station tab at `index` in the open panel. */
  function selectManualStation(doc, index) {
    const host = bodyOf(doc).children.find((c) => c.className === 'settings-documentation');
    const tabs = host.children.find((c) => c.className === 'manual-tabs');
    tabs.children[index].click();
  }

  it('keeps the reader on their station when the host pushes new debug state', () => {
    const doc = makeDoc();
    const inst = mount(doc, { getManual: () => TWO_STATION_MANUAL });
    inst.open();
    inst.selectTab('ship-manual');
    selectManualStation(doc, 1);
    expect(allText(bodyOf(doc))).toContain('Patch the ship.');

    // Exactly what client.html does when a DebugState frame arrives.
    inst.rebuildContent();
    expect(allText(bodyOf(doc))).toContain('Patch the ship.');
    expect(allText(bodyOf(doc))).not.toContain('Fly the ship.');
  });

  it('keeps the reader on their station across a Settings tab round trip', () => {
    const doc = makeDoc();
    const inst = mount(doc, { getManual: () => TWO_STATION_MANUAL });
    inst.open();
    inst.selectTab('ship-manual');
    selectManualStation(doc, 1);

    inst.selectTab('audio');
    inst.selectTab('ship-manual');
    expect(allText(bodyOf(doc))).toContain('Patch the ship.');
  });

  it('does not strand the reader past the end of a shorter manual', () => {
    const doc = makeDoc();
    let manual = TWO_STATION_MANUAL;
    const inst = mount(doc, { getManual: () => manual });
    inst.open();
    inst.selectTab('ship-manual');
    selectManualStation(doc, 1);

    // A one-station ship replaces the two-station one.
    manual = { stations: [{ station_id: 'helm', overview: 'Fly the ship.', sections: [] }] };
    inst.rebuildContent();
    expect(allText(bodyOf(doc))).toContain('Fly the ship.');
  });
});

// ── Painted from the host, never from the tap ────────────────────────────────
//
// The PASM decision `client-settings-menu-tabs` carries `must_not_be: painted
// from local optimism`, and nothing pinned it: the panel holds no local toggle
// state, so it cannot misbehave today — which is exactly the kind of invariant
// that gets refactored away by someone "fixing" the button's latency.

describe('a debug toggle waits for the host', () => {
  let doc;
  let sent;
  let state;

  beforeEach(() => {
    doc = makeDoc();
    sent = [];
    // The server HAS reported, with everything off, so the panel is painting
    // from truth rather than from "nothing has arrived yet".
    state = {
      stations: [],
      stationRatings: {},
      debugFlags: { flags: {}, godMode: false },
    };
  });

  const mountWatched = () =>
    mount(doc, {
      getState: () => state,
      send: (type, data) => sent.push({ type, data }),
    });

  const control = (id) =>
    bodyButtons(doc).find((b) => b.getAttribute('data-control') === id);

  it('sends on click but stays un-pressed until DebugState arrives', () => {
    const inst = mountWatched();
    inst.open();
    // Debug is now the LAST settings tab, so the panel opens on Audio; select
    // Debug to reach its controls (only the active tab's body is built).
    inst.selectTab('debug');

    const wireframes = control('wireframes');
    expect(wireframes.getAttribute('aria-pressed')).toBe('false');

    wireframes.click();

    // The tap went out…
    expect(sent).toEqual([{ type: 'ToggleDebugFlag', data: { flag: 'Regions' } }]);
    // …and the button did NOT move. Re-rendering changes nothing either: the
    // panel has no local answer to render, only the server's.
    expect(control('wireframes').getAttribute('aria-pressed')).toBe('false');
    expect(control('wireframes').classList.contains('active')).toBe(false);
    inst.rebuildContent();
    expect(control('wireframes').getAttribute('aria-pressed')).toBe('false');

    // Only the host's read-back presses it.
    state.debugFlags = { flags: { Regions: true }, godMode: false };
    inst.rebuildContent();
    expect(control('wireframes').getAttribute('aria-pressed')).toBe('true');
    expect(control('wireframes').classList.contains('active')).toBe(true);
  });

  it('does the same for god mode, which a demo build refuses outright', () => {
    const inst = mountWatched();
    inst.open();
    inst.selectTab('debug');

    control('godmode').click();
    expect(sent).toEqual([
      { type: 'ControlSystem', data: godModeMessage().data },
    ]);
    // A demo build compiles the route away and reports the flag unchanged, so
    // an optimistic press would claim a cheat the player never got.
    inst.rebuildContent();
    expect(control('godmode').getAttribute('aria-pressed')).toBe('false');

    state.debugFlags = { flags: {}, godMode: true };
    inst.rebuildContent();
    expect(control('godmode').getAttribute('aria-pressed')).toBe('true');
  });

  it('does the same for pause, which is its own message on its own route', () => {
    const inst = mountWatched();
    inst.open();
    inst.selectTab('gameplay');

    const pause = control(PAUSE_CONTROL_ID);
    expect(pause.getAttribute('aria-pressed')).toBe('false');
    pause.click();
    expect(sent).toEqual([{ type: 'TogglePause', data: undefined }]);
    inst.rebuildContent();
    expect(control(PAUSE_CONTROL_ID).getAttribute('aria-pressed')).toBe('false');

    // `paused` is its own field on the read-back, not an entry in `flags` —
    // pause is authoritative simulation state, not a debug overlay.
    state.debugFlags = { flags: {}, paused: true, godMode: false };
    inst.rebuildContent();
    expect(control(PAUSE_CONTROL_ID).getAttribute('aria-pressed')).toBe('true');
  });
});

// ── The release gate ─────────────────────────────────────────────────────────

describe('the demo build gate', () => {
  afterEach(() => { setBuildFlags({ demo: null }); });

  // Fails if the gate is inverted: a dev build MUST show Debug/Cheat and a
  // demo build MUST NOT, and this asserts both directions from one table.
  it('hides exactly the Debug/Cheat tab in a demo build and nothing in a dev build', () => {
    const dev = buildSettingsState({ demo: false }).tabs.map((tb) => tb.id);
    const demo = buildSettingsState({ demo: true }).tabs.map((tb) => tb.id);
    expect(dev).toEqual(TABS.concat(CLIENT_ACCESSIBILITY_TABS, CLIENT_DOCUMENTATION_TABS).map((tb) => tb.id));
    expect(demo).toEqual(
      TABS.filter((tb) => !tb.gated).concat(CLIENT_ACCESSIBILITY_TABS, CLIENT_DOCUMENTATION_TABS).map((tb) => tb.id),
    );
    expect(dev).toContain('debug');
    expect(demo).not.toContain('debug');
    // Audio and Gameplay are not build-gated — the demo needs both.
    expect(demo).toContain('audio');
    expect(demo).toContain('gameplay');
    // The Accessibility tab is phone-only and never gated — it must survive the
    // demo build (issue #1102 AC1).
    expect(dev).toContain('accessibility');
    expect(demo).toContain('accessibility');
    expect(demo).toContain('station-help');
    expect(demo).toContain('ship-manual');
  });

  it('falls back off a tab the build gated away instead of rendering nothing', () => {
    expect(buildSettingsState({ demo: true, activeTab: 'debug' }).activeTab).toBe('audio');
    expect(buildSettingsState({ demo: false, activeTab: 'debug' }).activeTab).toBe('debug');
  });

  it('reads the demo flag from the page meta tag the deploy stamps', () => {
    const withTag = (content) => ({
      querySelector: (sel) =>
        sel === 'meta[name="phoenix-build-demo"]'
          ? { getAttribute: () => content }
          : null,
    });
    expect(isDemoBuild({ doc: withTag('true'), win: {} })).toBe(true);
    expect(isDemoBuild({ doc: withTag('false'), win: {} })).toBe(false);
    // No tag at all is a DEV build — an unknown build must not silently hide
    // the debug tools during development.
    expect(isDemoBuild({ doc: { querySelector: () => null }, win: {} })).toBe(false);
  });

  it('builds no Debug tab body in a demo build', () => {
    const doc = makeDoc();
    const inst = mount(doc, { isDemo: () => true });
    inst.open();
    expect(tabBarOf(doc).children.map((c) => c.getAttribute('data-tab')))
      .toEqual(['audio', 'gameplay', 'controls', 'accessibility', 'station-help', 'ship-manual']);
    // …and nothing in the body offers a debug control.
    for (const entry of CLIENT_DEBUG_FLAGS) {
      expect(bodyButtons(doc).some((b) => b.getAttribute('data-control') === entry.id))
        .toBe(false);
    }
  });

  // ── Pause is gated per-control, not per-tab ────────────────────────────────
  //
  // The Gameplay tab itself ships in every build — the demo needs the rating,
  // QR and leave-station controls on it. Only the pause button goes, matching
  // `ClientMessage::TogglePause`, which a demo binary cannot even decode. The
  // reason is not tidiness: nothing server-side checks station, captaincy or
  // game phase before honouring a client pause, so in a demo any one of N
  // strangers could freeze the mission for everyone, repeatedly.

  it('offers pause in a dev build and not in a demo build', () => {
    expect(buildSettingsState({ demo: false }).showPause).toBe(true);
    expect(buildSettingsState({ demo: true }).showPause).toBe(false);
  });

  it('renders no pause control in a demo build, but keeps the Gameplay tab', () => {
    const withStation = {
      stations: [{ id: 'helm', holder_token: 'tok1', ratings: ['Std', 'Simplified'] }],
      stationRatings: { helm: 'Simplified' },
    };
    for (const demo of [false, true]) {
      const doc = makeDoc();
      const inst = mount(doc, { isDemo: () => demo, getState: () => withStation });
      inst.open();
      inst.selectTab('gameplay');
      const hasPause = bodyButtons(doc)
        .some((b) => b.getAttribute('data-control') === PAUSE_CONTROL_ID);
      expect(hasPause, `pause control present=${hasPause} for demo=${demo}`).toBe(!demo);
      // The rest of the tab is untouched either way — this is a control-level
      // gate, not the tab-level one Debug/Cheat gets.
      expect(bodyButtons(doc).some((b) => b.textContent === t('settings.toggle_qr')))
        .toBe(true);
      expect(bodyButtons(doc).some((b) => b.className.includes('settings-leave-btn')))
        .toBe(true);
    }
  });

  it('sends nothing that could pause when the demo build is mounted', () => {
    const doc = makeDoc();
    const sent = [];
    const inst = mount(doc, {
      isDemo: () => true,
      send: (type, data) => sent.push({ type, data }),
    });
    inst.open();
    inst.selectTab('gameplay');
    // Every button on the demo's Gameplay tab, clicked. None of them may be
    // the pause message — the host would refuse it, but the honest client
    // does not offer a control that cannot work.
    for (const btn of bodyButtons(doc)) btn.click();
    expect(sent.map((m) => m.type)).not.toContain('TogglePause');
    expect(sent.map((m) => m.type)).not.toContain('ToggleDebugFlag');
  });
});

// ── The page the cog lives on ────────────────────────────────────────────────

describe('client.html', () => {
  it('keeps the selected-gamepad disconnect alert outside the Settings modal', () => {
    expect(CLIENT_HTML).toMatch(
      /<div id="gamepad-input-alert" role="alert" aria-live="assertive"[^>]*hidden><\/div>/,
    );
    expect(CLIENT_HTML).toContain("t('client.gamepad.disconnect_warning')");
    expect(CLIENT_HTML).toContain('updateGamepadClientWarning(state)');
    expect(CLIENT_HTML).toContain('settingsPanel.updateGamepadState(state)');
  });

  it('retains and uses the gamepad runtime disposer while the transport is live', () => {
    expect(CLIENT_HTML).toContain(
      'isTransportLive: () => !!(activeLink && activeLink.connected)',
    );
    expect(CLIENT_HTML).toContain(
      'disposeGamepadInput = gamepadInputRuntime.start(window) || null',
    );
    expect(CLIENT_HTML).toMatch(/addEventListener\('pagehide',[\s\S]*?dispose\(\);[\s\S]*?\{ once: true \}\)/);
  });

  it('does not add a duplicate fixed station title above the active console', () => {
    expect(CLIENT_HTML).not.toMatch(/id="phase-title"/);
    expect(CLIENT_HTML).not.toMatch(/_consoleTitleEl/);
  });

  it('uses fullscreen and exit glyphs rather than a help glyph', () => {
    expect(CLIENT_HTML).toMatch(/id="fullscreen-btn"[^>]*>⛶<\/button>/);
    // The icon-sync logic itself now lives in gui/page-chrome.js (issue
    // #1227), shared with server.html — client.html only mounts it.
    expect(PAGE_CHROME_JS).toMatch(/doc\.fullscreenElement \? '✕' : '⛶'/);
  });

  const zIndexOf = (pattern) => {
    const m = CLIENT_HTML.match(pattern);
    expect(m, `pattern not found in client.html: ${pattern}`).not.toBeNull();
    return Number(m[1]);
  };

  // Issue #939 hit exactly this on the host page: a `position: fixed` cog is
  // only reachable if its z-index clears EVERY full-viewport panel that can be
  // on screen when the operator wants it — not just the one a manual check
  // happened to land on. On the phone that is four panels, and three of them
  // are pre-mission or post-mission states where the Audio and Gameplay tabs
  // are exactly what you would reach for.
  it('the cog outranks every full-viewport overlay it must sit above', () => {
    const btn = zIndexOf(/\.settings-btn\s*\{[^}]*z-index:\s*(\d+)/);
    const overlay = zIndexOf(/\.settings-overlay\s*\{[^}]*z-index:\s*(\d+)/);
    for (const [name, pattern] of [
      ['#coordination-popup', /#coordination-popup\s*\{[^}]*z-index:\s*(\d+)/],
      ['#waiting-overlay', /#waiting-overlay\s*\{[^}]*z-index:\s*(\d+)/],
      ['#scenario-picker-overlay', /#scenario-picker-overlay\s*\{[^}]*z-index:\s*(\d+)/],
      ['#game-over-overlay', /#game-over-overlay\s*\{[^}]*z-index:\s*(\d+)/],
    ]) {
      const panel = zIndexOf(pattern);
      expect(btn, `cog is buried under ${name}`).toBeGreaterThan(panel);
      expect(overlay, `panel is buried under ${name}`).toBeGreaterThan(panel);
    }
    // …but stays under the asset-loading screen, which is a genuine
    // "nothing is ready yet" state with nothing worth settling.
    expect(btn).toBeLessThan(zIndexOf(/#asset-loading\s*\{[^}]*z-index:\s*(\d+)/));
  });

  /**
   * The UNQUALIFIED `.settings-btn` rule — the page-body home, the one that
   * fixes the cog to the viewport corner.
   *
   * Anchored to the start of a line on purpose. Since issue #1372 the cog is
   * one node that MOVES: `#station-hero .settings-btn` turns the fixed corner
   * off again while the bar owns it, and a bare `.settings-btn {` search would
   * find that override first and measure a rectangle of `auto`s.
   */
  function settingsBtnRule() {
    const rule = CLIENT_HTML.match(/(?:^|\n)\s*\.settings-btn\s*\{([^}]*)\}/);
    expect(rule, '.settings-btn rule not found').not.toBeNull();
    return rule[1];
  }

  it('sits top-left, mirroring the host page rather than sharing #status corner', () => {
    const rule = settingsBtnRule();
    expect(rule).toMatch(/top:/);
    expect(rule).not.toMatch(/bottom:/);
  });

  // ── Clearance ─────────────────────────────────────────────────────────────
  //
  // Stacking is only half of "reachable". Issue #939 shipped a cog buried
  // under the panels it had to sit above; the opposite failure is a cog that
  // outranks everything and therefore sits ON the first glyphs of whatever is
  // underneath. The two tests below pin the second so a fix for one can never
  // reintroduce the other.
  //
  // HONESTLY, WHAT THIS SEES: it recomputes the cog's rectangle from the
  // `top`/`left`/`width`/`height` it declares and compares that against the
  // gutters the occluded surfaces declare. That is arithmetic over source
  // text, not layout. It cannot see a transform, an absolutely-positioned
  // child that escapes its container's padding box, or a rule in a file it
  // does not read. What it does catch is the regression that actually happened
  // here — a gutter tightened back under the cog — and it catches it in every
  // console at once.
  //
  // The root font-size is no longer a fixed browser default. Since issue #1422
  // the shell scales with the private profile's text setting
  // (`html { font-size: calc(var(--root-size-shell) * var(--a11y-text-scale)) }`),
  // so a `rem` in the cog's rule is worth anything from the shell baseline up
  // to that baseline times the supported ceiling. Clearance has to hold at the
  // size the operator can actually select, so the conversion below uses the
  // CEILING — the worst case — rather than a default nobody is pinned to. The
  // cog's four geometry properties happen to all be px today, which is why the
  // number does not currently move the result; the two assertions in `cogRect`
  // pin the source of both halves so it cannot drift silently.

  /** gui/tokens.css `--root-size-shell` — the shell's baseline root size. */
  const SHELL_ROOT_PX = 13;
  /** The largest a `rem` in the shell can be: baseline x the exposed ceiling. */
  const ROOT_FONT_PX = SHELL_ROOT_PX * TEXT_SCALE_MAX;

  /** The cog's rectangle in CSS px, read from client.html's own declarations. */
  function cogRect() {
    // A rem below is only worth ROOT_FONT_PX while these two hold: the token
    // still declares the baseline this assumes, and the shell still multiplies
    // exactly that token by the text scale.
    expect(
      TOKENS_CSS.match(new RegExp(`--root-size-shell:\\s*${SHELL_ROOT_PX}px`)),
      'gui/tokens.css moved --root-size-shell — the rem maths below is wrong',
    ).not.toBeNull();
    expect(
      CLIENT_HTML.match(
        /html\s*\{\s*font-size:\s*calc\(var\(--root-size-shell\)\s*\*\s*var\(--a11y-text-scale/,
      ),
      'client.html no longer scales the shell root from --root-size-shell',
    ).not.toBeNull();

    const rule = settingsBtnRule();
    const px = (prop) => {
      const m = rule.match(new RegExp(`${prop}:\\s*([\\d.]+)(rem|px)`));
      expect(m, `.settings-btn declares no ${prop}`).not.toBeNull();
      return Number(m[1]) * (m[2] === 'rem' ? ROOT_FONT_PX : 1);
    };
    const top = px('top');
    const left = px('left');
    return { right: left + px('width'), bottom: top + px('height') };
  }

  it('leaves the lobby header room rather than clipping #ship-name', () => {
    const m = CLIENT_HTML.match(
      /#lobby-ui\.active\s*\{[^}]*padding:\s*(\d+)px\s+\d+px\s+\d+px/,
    );
    expect(m, '#lobby-ui.active declares no padding shorthand').not.toBeNull();
    expect(
      Number(m[1]),
      'the lobby starts under the cog — #ship-name loses its first characters',
    ).toBeGreaterThanOrEqual(cogRect().bottom);
  });

  // The consoles used to reserve that corner too — `gui/console.css` held a
  // 44px left gutter on `body .panel-inner` in both orientations (issue #940),
  // because the cog floated over whichever console iframe was showing.
  //
  // Since issue #1372 the cog is a CHILD of the bar for the whole of play, so
  // it floats over no console at all, and issue #1374 took the gutter out: a
  // strip of a phone screen was being kept clear for a control that had left.
  // This is the inverse of the old assertion, and it is the one that matters
  // now — the gutter cost every console 44px invisibly, so its return would be
  // just as easy to miss as its absence once was.
  it('no console reserves a cog gutter any more', () => {
    const reservations = CONSOLE_CSS.match(/body\s+\.panel-inner\s*\{[^}]*\}/g) || [];
    expect(
      reservations,
      'gui/console.css reserves the cog corner again — the cog has been a bar '
        + 'child since #1372 and floats over no console, so this is 44px of '
        + 'every phone screen kept clear for nothing',
    ).toEqual([]);
  });

  // ── The bar owns the chrome in game (issue #1372) ─────────────────────────
  //
  // The clearance tests above pin where the cog sits while it is a page-body
  // control. These pin the other half: that it stops being one for the
  // duration of play, and that the help button travels with it.

  it('declares the help button beside the cog with no English of its own', () => {
    const btn = CLIENT_HTML.match(/<button id="help-btn"[\s\S]*?<\/button>/);
    expect(btn, 'client.html declares no #help-btn').not.toBeNull();
    // Same class, so it takes the cog's size, corner and stacking in one go.
    expect(btn[0]).toMatch(/class="settings-btn help-btn"/);
    expect(btn[0]).toMatch(/aria-haspopup="dialog"/);
    // Its label is a string id, not a word: the glyph is all the markup holds.
    expect(btn[0]).toMatch(/data-i18n-attr="[^"]*title:client\.hero\.help\.label/);
    expect(btn[0]).toMatch(/data-i18n-attr="[^"]*aria-label:client\.hero\.help\.label/);
  });

  it('routes the help button to the Station Help tab of the one dialog', () => {
    expect(CLIENT_HTML).toContain("settingsPanel.openTab('station-help')");
  });

  it('moves the one cog into the bar rather than hiding a second copy', () => {
    // The page asks the overlay shell to move its own button — it never
    // reaches for #settings-btn itself, which is what keeps the focus trap,
    // the toggle and aria-expanded on one node.
    expect(CLIENT_HTML).toContain('settingsPanel.setButtonContainer(');
    expect(CLIENT_HTML).not.toMatch(/getElementById\('settings-btn'\)/);
    // Cog first (inserted before the tab list), help last (appended).
    expect(CLIENT_HTML).toMatch(
      /setButtonContainer\(inBar \? bar : null, inBar \? tabsEl : null\)/,
    );
    expect(CLIENT_HTML).toMatch(/if \(help\.parentNode !== host\) host\.appendChild\(help\)/);
  });

  it('weighs the game-over and pre-play surfaces, not just "in game"', () => {
    // vis.game stays true through GameOver and the pre-play surfaces cover a
    // live bar, so the slot decision reads all three facts.
    expect(CLIENT_HTML).toMatch(
      /heroChromeSlot\(\{ heroVisible: heroBarVisible, prePlaySurface, gameOverVisible \}\)/,
    );
    expect(CLIENT_HTML).toMatch(/prePlaySurface = view\.surface \|\| null;/);
    expect(CLIENT_HTML).toMatch(/gameOverVisible = !!gv\.visible;/);
  });

  it('turns the fixed corner off while the bar holds the chrome', () => {
    const rule = CLIENT_HTML.match(/#station-hero \.settings-btn\s*\{([^}]*)\}/);
    expect(rule, 'no in-bar rule for the cog').not.toBeNull();
    expect(rule[1]).toMatch(/position:\s*relative/);
    expect(rule[1]).toMatch(/top:\s*auto/);
    expect(rule[1]).toMatch(/left:\s*auto/);
  });

  it('draws 40px of ink and offers 44px of target', () => {
    const rule = settingsBtnRule();
    expect(rule).toMatch(/width:\s*40px/);
    // The drawn box is narrower than the platform floor on purpose — two of
    // these plus the tabs have to cross a 390px phone — so the target is
    // widened behind it, the escape ph-console-styles.js documents.
    const target = CLIENT_HTML.match(/\.settings-btn::after\s*\{([^}]*)\}/);
    expect(target, 'no expanded touch target behind the cog').not.toBeNull();
    expect(target[1]).toMatch(/width:\s*max\(100%,\s*var\(--control-hit-min\)\)/);
    expect(target[1]).toMatch(/height:\s*max\(100%,\s*var\(--control-hit-min\)\)/);
  });

  // The client has no WASM, so the meta tag is the ONLY thing that can tell it
  // which build it is. It must ship saying "not the demo" — deploy-demo.yml
  // rewrites it and fails the run if the rewrite does not land.
  it('carries the build-flag meta tag, defaulting to a dev build', () => {
    expect(CLIENT_HTML).toMatch(
      /<meta\s+name="phoenix-build-demo"\s+content="false"\s*\/?>/,
    );
  });
});

// ── The debug-toggle message shape ───────────────────────────────────────────

describe('the client → server debug messages', () => {
  it('builds a ToggleDebugFlag matching the pinned Rust wire shape', () => {
    // Pinned by codec::client_settings_menu_wire_shapes_are_pinned.
    expect(debugFlagMessage('Modifiers')).toEqual({
      type: 'ToggleDebugFlag',
      data: { flag: 'Modifiers' },
    });
    expect(debugFlagMessage('Regions').data.flag).toBe('Regions');
  });

  it('builds TogglePause as a unit variant carrying no data at all', () => {
    // A unit variant on the wire: `{"type":"TogglePause"}`. `data: {}` is a
    // DIFFERENT message that the host rejects, which is why the builder omits
    // the key rather than sending an empty object — both pinned by
    // codec::client_settings_menu_wire_shapes_are_pinned.
    expect(pauseMessage()).toEqual({ type: 'TogglePause' });
    expect('data' in pauseMessage()).toBe(false);
  });

  it('refuses to build a malformed flag message rather than sending junk', () => {
    expect(() => debugFlagMessage('')).toThrow(TypeError);
    expect(() => debugFlagMessage(undefined)).toThrow(TypeError);
  });

  it('builds God Mode as a ControlSystem envelope on the ownerless god-mode id', () => {
    // God Mode is the one client-reachable toggle that changes simulation
    // outcomes, so it crosses command admission (issue #900) rather than
    // riding the session route the overlays use.
    expect(godModeMessage()).toEqual({
      type: 'ControlSystem',
      data: { target: GOD_MODE_SYSTEM_ID, payload: { type: 'ToggleGodMode' } },
    });
    expect(GOD_MODE_SYSTEM_ID).toBe('god-mode');
  });

  it('sends one ToggleDebugFlag per debug button, with that button\'s flag', () => {
    const doc = makeDoc();
    const sent = [];
    const inst = mount(doc, { send: (type, data) => sent.push({ type, data }) });
    inst.open();
    inst.selectTab('debug');
    for (const entry of CLIENT_DEBUG_FLAGS) {
      const btn = bodyButtons(doc).find((b) => b.getAttribute('data-control') === entry.id);
      expect(btn, `no button for ${entry.id}`).toBeDefined();
      btn.click();
    }
    expect(sent.map((m) => m.type)).toEqual(CLIENT_DEBUG_FLAGS.map(() => 'ToggleDebugFlag'));
    expect(sent.map((m) => m.data.flag)).toEqual(CLIENT_DEBUG_FLAGS.map((e) => e.flag));
  });

  it('sends ControlSystem for the God Mode cheat button', () => {
    const doc = makeDoc();
    const sent = [];
    const inst = mount(doc, { send: (type, data) => sent.push({ type, data }) });
    inst.open();
    inst.selectTab('debug');
    bodyButtons(doc).find((b) => b.getAttribute('data-control') === 'godmode').click();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toEqual(godModeMessage());
  });

  it('sends TogglePause — not a debug flag — from the Gameplay tab', () => {
    const doc = makeDoc();
    const sent = [];
    const inst = mount(doc, { send: (type, data) => sent.push({ type, data }) });
    inst.open();
    inst.selectTab('gameplay');
    bodyButtons(doc).find((b) => b.getAttribute('data-control') === PAUSE_CONTROL_ID).click();
    expect(sent).toEqual([{ type: 'TogglePause', data: undefined }]);
    // The point of the split: pause no longer rides `ToggleDebugFlag`, so that
    // message and its whole server-side drain can be compiled out of a demo
    // build without taking the host's pause with them.
    expect(sent.map((m) => m.type)).not.toContain('ToggleDebugFlag');
  });
});

// ── The fold-in state builder ────────────────────────────────────────────────

describe('buildSettingsState', () => {
  const state = (extra) => ({
    stations: [
      { id: 'helm', name: 'Helm', holder_token: 'tok1', ratings: ['Std', 'Simplified'] },
    ],
    stationRatings: { helm: 'Simplified' },
    ...extra,
  });

  it('paints debug buttons OFF and un-reported before the first DebugState', () => {
    const view = buildSettingsState({ state: state(), myToken: 'tok1', demo: false });
    expect(view.reported).toBe(false);
    expect(view.debugFlags.every((f) => f.on === false)).toBe(true);
    expect(view.godMode).toBe(false);
    expect(view.paused).toBe(false);
  });

  it('paints from the server read-back, not from what was clicked', () => {
    const view = buildSettingsState({
      state: state({
        debugFlags: { flags: { Regions: true }, paused: true, godMode: true },
      }),
      myToken: 'tok1',
      demo: false,
    });
    expect(view.reported).toBe(true);
    const byFlag = Object.fromEntries(view.debugFlags.map((f) => [f.flag, f.on]));
    expect(byFlag.Regions).toBe(true);
    // Reported false / absent are both OFF — the panel never guesses.
    expect(byFlag.Damage).toBe(false);
    expect(view.godMode).toBe(true);
    expect(view.paused).toBe(true);
  });

  it('resolves the active rating and labels it from the derived string id', () => {
    const view = buildSettingsState({ state: state(), myToken: 'tok1', demo: false });
    expect(view.stationId).toBe('helm');
    expect(view.ratings.map((r) => r.name)).toEqual(['Std', 'Simplified']);
    expect(view.ratings.find((r) => r.active).name).toBe('Simplified');
    expect(view.ratings[0].label).toBe(t('station.rating.std.name'));
  });

  it('reports no station when the player holds none', () => {
    const view = buildSettingsState({
      state: { stations: [], stationRatings: {} },
      myToken: 'tok1',
      demo: false,
    });
    expect(view.stationId).toBeNull();
    expect(view.ratings).toEqual([]);
  });

  it('exposes the local player afk flag from the roster (issue #1104)', () => {
    // Painted from server truth on the roster, never from what was clicked.
    const away = buildSettingsState({
      state: state({ players: [{ token: 'tok1', afk: true }] }),
      myToken: 'tok1',
      demo: false,
    });
    expect(away.afk).toBe(true);

    const present = buildSettingsState({
      state: state({ players: [{ token: 'tok1', afk: false }] }),
      myToken: 'tok1',
      demo: false,
    });
    expect(present.afk).toBe(false);

    // A silent/legacy roster (no players, or no matching token) defaults false.
    expect(buildSettingsState({ state: state(), myToken: 'tok1', demo: false }).afk).toBe(false);
  });

  it('folds a DebugState message through SimState into the shape it consumes', () => {
    // The two halves of the read-back path meet here: sim-state.js folds the
    // wire message, buildSettingsState reads what it produced.
    const sim = new ClientSimState();
    sim.apply({
      type: 'DebugState',
      data: {
        flags: [['Regions', true], ['Modifiers', false]],
        paused: true,
        god_mode: true,
      },
    });
    expect(sim.debugFlags).toEqual({
      flags: { Regions: true, Modifiers: false },
      paused: true,
      godMode: true,
    });
    const view = buildSettingsState({
      state: { stations: [], stationRatings: {}, debugFlags: sim.debugFlags },
      myToken: 'tok1',
      demo: false,
    });
    expect(view.debugFlags.find((f) => f.flag === 'Regions').on).toBe(true);
    expect(view.debugFlags.find((f) => f.flag === 'Modifiers').on).toBe(false);
    expect(view.paused).toBe(true);
    expect(view.godMode).toBe(true);
  });

  it('keeps the host flags across a Welcome reset', () => {
    // The host re-announces when a peer identifies, and that broadcast flushes
    // in the same frame as that peer's own Welcome. Clearing the mirror on
    // Welcome would race it — and these are the HOST's flags, which do not
    // change because a world loaded.
    const sim = new ClientSimState();
    sim.apply({ type: 'DebugState', data: { flags: [['Regions', true]], god_mode: false } });
    sim.reset();
    expect(sim.debugFlags.flags.Regions).toBe(true);
  });

  it('ignores a malformed flags list rather than throwing at the fold', () => {
    const sim = new ClientSimState();
    sim.apply({ type: 'DebugState', data: { flags: [['Regions', true], 'junk', []] } });
    expect(sim.debugFlags).toEqual({ flags: { Regions: true }, paused: false, godMode: false });
  });
});

// ── The Gameplay tab's session controls (pre-#940 behaviour, retabbed) ───────

describe('gameplay tab — rating, QR and leave station', () => {
  let doc, sent, inst;

  const withStation = {
    stations: [
      { id: 'helm', name: 'Helm', holder_token: 'tok1', ratings: ['Std', 'Simplified'] },
    ],
    stationRatings: { helm: 'Simplified' },
  };

  function openGameplay(state) {
    doc = makeDoc();
    sent = [];
    inst = mount(doc, {
      send: (type, data) => sent.push({ type, data }),
      getState: () => state,
    });
    inst.open();
    inst.selectTab('gameplay');
  }

  it('renders a rating button per rating with the active one marked', () => {
    openGameplay(withStation);
    const rating = bodyButtons(doc).filter((b) =>
      String(b.getAttribute('data-control') || '').startsWith('rating-'));
    expect(rating).toHaveLength(2);
    const active = rating.filter((b) => b.classList.contains('active'));
    expect(active).toHaveLength(1);
    expect(active[0].getAttribute('data-control')).toBe('rating-Simplified');
  });

  it('sends SetStationRating when a different rating is picked', () => {
    openGameplay(withStation);
    bodyButtons(doc).find((b) => b.getAttribute('data-control') === 'rating-Std').click();
    expect(sent).toEqual([{ type: 'SetStationRating', data: { rating_name: 'Std' } }]);
  });

  it('ignores a click on the rating already active', () => {
    openGameplay(withStation);
    bodyButtons(doc).find((b) => b.getAttribute('data-control') === 'rating-Simplified').click();
    expect(sent).toHaveLength(0);
  });

  it('hides the rating row when the station offers only one rating', () => {
    openGameplay({
      stations: [{ id: 'helm', holder_token: 'tok1', ratings: ['Std'] }],
      stationRatings: { helm: 'Std' },
    });
    expect(bodyButtons(doc).some((b) =>
      String(b.getAttribute('data-control') || '').startsWith('rating-'))).toBe(false);
  });

  it('sends ToggleQrCode from the QR button, with no data at all', () => {
    // The absent `data` is the assertion (issue #1329). A browser host reads
    // this frame in JavaScript, but a native host DECODES it — into a unit
    // variant, which `{"data":{}}` is not — so the shape here decides whether
    // the same button works against the same crew's other host.
    openGameplay(withStation);
    const qr = bodyButtons(doc).find((b) => b.textContent === t('settings.toggle_qr'));
    expect(qr).toBeDefined();
    qr.click();
    expect(sent).toEqual([{ type: 'ToggleQrCode', data: undefined }]);
  });

  it('sends ReleaseStation and closes when Leave Station is used', () => {
    openGameplay(withStation);
    const leave = bodyButtons(doc).find((b) => b.className.includes('settings-leave-btn'));
    expect(leave.textContent).toBe(t('settings.leave_station'));
    leave.click();
    expect(sent[0].type).toBe('ReleaseStation');
    expect(findOverlay(doc).hidden).toBe(true);
  });

  it('hides Leave Station when the player holds no station', () => {
    openGameplay({ stations: [], stationRatings: {} });
    expect(bodyButtons(doc).some((b) => b.className.includes('settings-leave-btn'))).toBe(false);
  });

  // ── AFK toggle (issue #1104) ───────────────────────────────────────────────

  const withStationAfk = (afk) => ({
    ...withStation,
    players: [{ token: 'tok1', afk }],
  });

  it('sends SetAfk with the flipped flag when the AFK toggle is used', () => {
    openGameplay(withStationAfk(false));
    const afkBtn = bodyButtons(doc).find((b) => b.getAttribute('data-control') === 'afk-toggle');
    expect(afkBtn).toBeDefined();
    afkBtn.click();
    expect(sent).toEqual([{ type: 'SetAfk', data: { afk: true } }]);
  });

  it('paints the AFK toggle pressed from the roster afk flag and leaves on tap', () => {
    openGameplay(withStationAfk(true));
    const afkBtn = bodyButtons(doc).find((b) => b.getAttribute('data-control') === 'afk-toggle');
    expect(afkBtn.getAttribute('aria-pressed')).toBe('true');
    expect(afkBtn.textContent).toBe(t('settings.afk_active'));
    afkBtn.click();
    expect(sent).toEqual([{ type: 'SetAfk', data: { afk: false } }]);
    // The panel stays open so the player can return the same way they left.
    expect(findOverlay(doc).hidden).toBe(false);
  });

  it('hides the AFK toggle when the player holds no station', () => {
    openGameplay({ stations: [], stationRatings: {}, players: [{ token: 'tok1', afk: false }] });
    expect(bodyButtons(doc).some((b) => b.getAttribute('data-control') === 'afk-toggle')).toBe(false);
  });

  it('offers no exit-to-lobby — that authority stays host-side', () => {
    openGameplay(withStation);
    // A phone may only request a return at GameOver, which the game-over
    // overlay already does. #940 must not add a second, unrestricted path.
    expect(bodyButtons(doc).some((b) => b.textContent === t('settings.gameplay.exit_to_lobby')))
      .toBe(false);
  });

  it('labels the pause control from the reported pause state', () => {
    openGameplay(withStation);
    expect(bodyButtons(doc)
      .find((b) => b.getAttribute('data-control') === PAUSE_CONTROL_ID).textContent)
      .toBe(t('settings.gameplay.pause'));

    openGameplay({ ...withStation, debugFlags: { flags: {}, paused: true, godMode: false } });
    const paused = bodyButtons(doc)
      .find((b) => b.getAttribute('data-control') === PAUSE_CONTROL_ID);
    expect(paused.textContent).toBe(t('settings.gameplay.resume'));
    expect(paused.getAttribute('aria-pressed')).toBe('true');
  });
});

// ── The Accessibility tab (issue #1102) ──────────────────────────────────────
//
// The tab is phone-only and its whole point is that NOTHING it does reaches the
// host: a presentation choice is client-local (AC5). Every control writes on
// the dedicated `onAccessibility` path, never `send`, and this pins that so a
// later refactor cannot quietly route a setting onto the wire.

describe('semantic controls tab', () => {
  const descendants = (root) => {
    const out = [];
    for (const child of root.children || []) {
      out.push(child, ...descendants(child));
    }
    return out;
  };

  function openControls(extra = {}) {
    const doc = makeDoc();
    const registry = createCaptainActionRegistry();
    const sent = [];
    const inst = mount(doc, {
      send: (type, data) => sent.push({ type, data }),
      getSemanticActions: () => registry.list(),
      onSemanticBinding: (actionId, slot, binding, options) =>
        registry.setBinding(actionId, slot, binding, options),
      onSemanticResetAction: (actionId) => registry.resetAction(actionId),
      onSemanticResetAll: () => registry.resetAllBindings(),
      ...extra,
    });
    inst.open();
    inst.selectTab('controls');
    return { doc, registry, sent, inst };
  }

  it('recognizes physical modifier codes and key-name fallbacks', () => {
    expect(isSemanticModifierEvent({ code: 'ControlRight', key: 'Control' })).toBe(true);
    expect(semanticModifierCode({ code: '', key: 'Shift' })).toBe('ShiftLeft');
    expect(semanticModifierCode({ code: 'KeyR', key: 'r' })).toBeNull();
  });

  it('shows both binding slots and their display/accessibility metadata', () => {
    const { doc } = openControls();
    const text = allText(bodyOf(doc));
    expect(text).toContain(t('semantic_action.captain.red_alert.label'));
    expect(text).toContain(t('semantic_action.captain.red_alert.accessibility'));
    const captures = descendants(bodyOf(doc)).filter((el) =>
      el.getAttribute && String(el.getAttribute('data-control') || '')
        .startsWith('semantic-binding-captain.red-alert-'));
    expect(captures).toHaveLength(2);
    expect(captures.map((el) => el.value)).toEqual([
      'R', t('input.gamepad.face_bottom'),
    ]);
    expect(captures.every((el) => el.getAttribute('aria-label'))).toBe(true);
  });

  it('exports and imports the portable private profile with accessible status', async () => {
    const downloads = [];
    const imported = [];
    const { doc } = openControls({
      onOperatorProfileExport: () => '{"kind":"project-phoenix/operator-profile"}',
      downloadOperatorProfile: (_doc, name, text) => {
        downloads.push({ name, text });
        return true;
      },
      readOperatorProfileFile: async (file) => file.contents,
      onOperatorProfileImport: async (text) => {
        imported.push(text);
        return {
          status: 'imported',
          diagnostics: [{ code: 'private-or-unsupported-fields-ignored' }],
        };
      },
    });
    const exportButton = bodyButtons(doc)
      .find((button) => button.textContent === t('settings.controls.profile.export'));
    exportButton.click();
    expect(downloads).toEqual([{
      name: 'phoenix-operator-profile.json',
      text: '{"kind":"project-phoenix/operator-profile"}',
    }]);
    expect(descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'operator-profile-status').textContent)
      .toBe(t('settings.controls.profile.status_exported'));

    const file = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'operator-profile-file');
    file.files = [{ contents: '{"version":1}' }];
    file.dispatch('change');
    await Promise.resolve();
    await Promise.resolve();
    expect(imported).toEqual(['{"version":1}']);
    const status = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'operator-profile-status');
    expect(status.textContent).toBe(t('settings.controls.profile.status_imported_normalized'));
    expect(status.getAttribute('role')).toBe('status');
  });

  it('maps profile refusals to assertive, explicit diagnostics', () => {
    expect(operatorProfileStatusView({ status: 'rejected', code: 'profile-version' }))
      .toEqual({ labelId: 'settings.controls.profile.status_refused_version', alert: true });
    expect(operatorProfileStatusView({ status: 'rejected', code: 'binding-conflict' }))
      .toEqual({ labelId: 'settings.controls.profile.status_refused_controls', alert: true });
    expect(operatorProfileStatusView({ status: 'rejected', code: 'profile-json' }))
      .toEqual({ labelId: 'settings.controls.profile.status_refused_corrupt', alert: true });
  });

  it('renders accessible continuous tuning controls and updates only client-local tuning', () => {
    const registry = createClientSemanticActionRegistry();
    const tuningChanges = [];
    const { doc, sent, inst } = openControls({
      getSemanticActions: () => registry.list(),
      onSemanticBinding: (actionId, slot, binding, options) =>
        registry.setBinding(actionId, slot, binding, options),
      onSemanticTuning: (actionId, tuning) => {
        tuningChanges.push({ actionId, tuning });
        return registry.setTuning(actionId, tuning);
      },
    });
    const deadzone = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-tuning-deadzone-helm.steering');
    const inverted = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-tuning-inverted-helm.steering');
    expect(deadzone.value).toBe('0.1');
    expect(deadzone.getAttribute('aria-label')).toBe(t('settings.controls.gamepad.deadzone'));
    expect(inverted.checked).toBe(false);
    expect(inverted.getAttribute('aria-label')).toBe(t('settings.controls.gamepad.inverted'));

    deadzone.focus();
    inst.updateGamepadState({
      devices: [{ index: 0, supported: true }], selectedIndex: 0,
      status: 'neutral', capturing: null,
    });
    expect(doc.activeElement).toBe(deadzone);
    expect(descendants(bodyOf(doc))).toContain(deadzone);

    deadzone.value = '0.2';
    deadzone.dispatch('input');
    inverted.checked = true;
    inverted.dispatch('change');
    expect(tuningChanges).toEqual([
      { actionId: 'helm.steering', tuning: { deadzone: 0.2 } },
      { actionId: 'helm.steering', tuning: { inverted: true } },
    ]);
    expect(registry.tuningProfile()['helm.steering']).toEqual({
      deadzone: 0.2, inverted: true,
    });
    expect(sent).toEqual([]);
  });

  it('keeps continuous axis capture focused across status updates and accepts an undirected axis', () => {
    const registry = createClientSemanticActionRegistry();
    let gamepad = {
      devices: [{ index: 0, supported: true }], selectedIndex: 0,
      status: 'ready', capturing: null,
    };
    const captures = [];
    const { doc, inst } = openControls({
      getSemanticActions: () => registry.list(),
      onSemanticBinding: (actionId, slot, binding, options) =>
        registry.setBinding(actionId, slot, binding, options),
      getGamepadState: () => gamepad,
      onSemanticCapture: (actionId, slot, active) => {
        captures.push({ actionId, slot, active });
        gamepad = {
          ...gamepad,
          status: active ? 'neutral' : 'ready',
          capturing: active ? { actionId, slot } : null,
        };
      },
    });
    const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-helm.steering-1');
    capture.focus();
    expect(capture.value).toBe(t('settings.controls.press_axis'));
    capture.dispatch('keydown', {
      code: 'KeyY', key: 'y', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    expect(registry.action('helm.steering').bindings[1]).toBeNull();
    inst.updateGamepadState(gamepad);
    expect(doc.activeElement).toBe(capture);
    expect(descendants(bodyOf(doc))).toContain(capture);

    inst.proposeSemanticBinding('helm.steering', 1, {
      type: 'gamepad', input: 'axis', control: 'right-stick-y',
    });
    expect(captures.at(-1)).toEqual({ actionId: 'helm.steering', slot: 1, active: false });
    expect(registry.action('helm.steering').bindings[1]).toEqual({
      type: 'gamepad', input: 'axis', control: 'right-stick-y',
    });
  });

  it('shows explicit device selection and accessible unsupported/disconnect states', () => {
    let gamepad = {
      devices: [
        { index: 0, supported: true },
        { index: 1, supported: false },
      ],
      selectedIndex: null,
      status: 'none',
      capturing: null,
    };
    const selections = [];
    const { doc, inst } = openControls({
      getGamepadState: () => gamepad,
      onGamepadSelection: (index) => selections.push(index),
    });
    let selector = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-gamepad-select');
    expect(selector.children.map((option) => option.value)).toEqual(['', '0', '1']);
    expect(selector.children[2].disabled).toBe(true);
    selector.value = '0';
    selector.dispatch('change');
    expect(selections).toEqual([0]);

    gamepad = {
      devices: [{ index: 1, supported: true }],
      selectedIndex: 0,
      status: 'disconnected',
      capturing: null,
    };
    inst.rebuildContent();
    const status = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-gamepad-status');
    expect(status.getAttribute('role')).toBe('alert');
    expect(status.getAttribute('aria-live')).toBe('assertive');
    expect(status.textContent).toBe(t('settings.controls.gamepad.status_disconnected'));
    selector = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-gamepad-select');
    expect(selector.children.some((option) => option.value === '0')).toBe(true);
  });

  it('shows native capability loss while retaining portable gamepad and vibration choices', () => {
    const { doc } = openControls({
      getOperatorCapabilities: () => ({
        surface: 'native-pane', keyboard: true, gamepad: false,
        vibration: false, semanticCues: true, accessibility: true,
      }),
      getGamepadState: () => ({
        devices: [], selectedIndex: null, retainedIndex: 2,
        status: 'unavailable', capturing: null,
      }),
    });
    const controls = descendants(bodyOf(doc));
    const selector = controls.find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-gamepad-select');
    const status = controls.find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-gamepad-status');
    const vibration = controls.find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'operator-profile-vibration-unavailable');

    expect(selector.disabled).toBe(true);
    expect(selector.value).toBe('2');
    expect(selector.children.at(-1).textContent)
      .toBe(t('settings.controls.gamepad.device_retained', { slot: '3' }));
    expect(status.textContent).toBe(t('settings.controls.gamepad.status_unavailable'));
    expect(status.getAttribute('role')).toBe('status');
    expect(vibration.textContent)
      .toBe(t('settings.controls.profile.vibration_unavailable'));
  });

  it('preserves exact binding focus across pad status changes and disarms on conflict/blur', () => {
    let gamepad = {
      devices: [{ index: 0, supported: true }],
      selectedIndex: 0,
      status: 'ready',
      capturing: null,
    };
    const captures = [];
    let inst = null;
    const opened = openControls({
      getGamepadState: () => gamepad,
      onSemanticCapture: (actionId, slot, active) => {
        captures.push({ actionId, slot, active });
        gamepad = {
          ...gamepad,
          status: active ? 'neutral' : 'ready',
          capturing: active ? { actionId, slot } : null,
        };
        if (inst) inst.updateGamepadState(gamepad);
      },
    });
    ({ inst } = opened);
    const { doc, registry } = opened;
    const control = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-1');
    control.focus();
    expect(doc.activeElement).toBe(control);
    expect(descendants(bodyOf(doc))).toContain(control);
    expect(control.value).toBe(t('settings.controls.press_key'));
    const status = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-gamepad-status');
    expect(status.textContent).toBe(t('settings.controls.gamepad.status_neutral'));

    // A keyboard remap still completes while the selected-pad status changes.
    control.dispatch('keydown', {
      code: 'KeyY', key: 'y', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[1]).toMatchObject({
      type: 'keyboard', code: 'KeyY',
    });
    expect(captures.at(-1)).toEqual({
      actionId: 'captain.red-alert', slot: 1, active: false,
    });

    // A conflicting key moves focus to Cancel only after capture is disarmed.
    const replacement = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-1');
    replacement.focus();
    replacement.dispatch('keydown', {
      // `captain.view`'s default slot-0 key, so this collides. It was the
      // Weapons Hold's KeyH until issue #1398 retired that action.
      code: 'KeyV', key: 'v', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    const cancel = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-conflict-cancel');
    expect(doc.activeElement).toBe(cancel);
    expect(captures.at(-1).active).toBe(false);

    // Cancel deliberately returns to a newly focused capture; leaving it is
    // the final disarm/neutral boundary.
    cancel.click();
    const retried = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-1');
    expect(doc.activeElement).toBe(retried);
    expect(descendants(bodyOf(doc))).toContain(retried);
    expect(captures.at(-1).active).toBe(true);
    retried.blur();
    expect(captures.at(-1).active).toBe(false);
  });

  it('captures a remap in memory without sending a ClientMessage', () => {
    const { doc, registry, sent } = openControls();
    const capture = descendants(bodyOf(doc)).find((el) =>
      el.getAttribute && el.getAttribute('data-control')
        === 'semantic-binding-captain.red-alert-0');
    let prevented = false;
    let stopped = false;
    capture.dispatch('keydown', {
      code: 'KeyY', ctrlKey: false, shiftKey: false, altKey: false, metaKey: false,
      repeat: false,
      preventDefault() { prevented = true; },
      stopPropagation() { stopped = true; },
    });
    expect(prevented).toBe(true);
    expect(stopped).toBe(true);
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0].code).toBe('KeyY');
    expect(sent).toEqual([]);
    const repainted = descendants(bodyOf(doc)).find((el) =>
      el.getAttribute && el.getAttribute('data-control')
        === 'semantic-binding-captain.red-alert-0');
    expect(repainted.value).toBe('Y');
  });

  it('offers Cancel or atomic Replace for an overlapping-context conflict', () => {
    const { doc, registry, sent } = openControls();
    const captureFor = (actionId, slot) => descendants(bodyOf(doc)).find((el) =>
      el.getAttribute && el.getAttribute('data-control')
        === `semantic-binding-${actionId}-${slot}`);
    captureFor(CAPTAIN_VIEW_ACTION_ID, 0).dispatch('keydown', {
      code: 'KeyR', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });

    const cancel = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-conflict-cancel');
    const replace = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-conflict-replace');
    expect(cancel).toBeDefined();
    expect(replace).toBeDefined();
    expect(doc.activeElement).toBe(cancel);
    expect(allText(bodyOf(doc))).toContain(t('semantic_action.captain.red_alert.label'));
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0].code).toBe('KeyR');
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0].code).toBe('KeyV');

    cancel.click();
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0].code).toBe('KeyR');
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0].code).toBe('KeyV');

    captureFor(CAPTAIN_VIEW_ACTION_ID, 0).dispatch('keydown', {
      code: 'KeyR', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-conflict-replace').click();
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0]).toBeNull();
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0].code).toBe('KeyR');
    expect(sent).toEqual([]);
  });

  it('presents every conflicting action and slot returned by the registry', () => {
    const doc = makeDoc();
    const registry = createSemanticActionRegistry();
    const definition = (id, contexts, code, labelId) => ({
      id,
      contexts,
      labelId,
      accessibilityLabelId: 'semantic_action.captain.red_alert.accessibility',
      bindings: [{ code }, null],
    });
    registry.register(definition(
      'multi.target', ['captain', 'bridge'], 'KeyA',
      'semantic_action.captain.red_alert.label',
    ));
    registry.register(definition(
      'captain.source', ['captain'], 'KeyB',
      'semantic_action.captain.view.label',
    ));
    registry.register(definition(
      'bridge.source', ['bridge'], 'KeyC',
      'semantic_action.captain.red_alert.label',
    ));
    registry.setBinding('captain.source', 0, { code: 'KeyY' });
    registry.setBinding('bridge.source', 1, { code: 'KeyY' });
    const inst = mount(doc, {
      getSemanticActions: () => registry.list(),
      onSemanticBinding: (actionId, slot, binding, options) =>
        registry.setBinding(actionId, slot, binding, options),
    });
    inst.open();
    inst.selectTab('controls');
    descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-multi.target-0')
      .dispatch('keydown', {
        code: 'KeyY', repeat: false, preventDefault() {}, stopPropagation() {},
      });

    const items = descendants(bodyOf(doc)).filter((el) =>
      el.className === 'settings-binding-conflict-item');
    expect(items).toHaveLength(2);
    expect(items.map((item) => item.textContent)).toEqual([
      t('settings.controls.conflict_item', {
        action: t('semantic_action.captain.view.label'), slot: '1',
      }),
      t('settings.controls.conflict_item', {
        action: t('semantic_action.captain.red_alert.label'), slot: '2',
      }),
    ]);
  });

  it('refuses the completed Ctrl+R chord, refocuses capture, then accepts an immediate retry', () => {
    const { doc, registry } = openControls();
    const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-0');
    let modifierPrevented = false;
    let modifierStopped = false;
    capture.dispatch('keydown', {
      code: 'ControlLeft', key: 'Control', ctrlKey: true, repeat: false,
      preventDefault() { modifierPrevented = true; },
      stopPropagation() { modifierStopped = true; },
    });
    expect(modifierPrevented).toBe(true);
    expect(modifierStopped).toBe(true);
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0].code).toBe('KeyR');
    expect(descendants(bodyOf(doc))).toContain(capture);

    capture.dispatch('keydown', {
      code: 'KeyR', key: 'r', ctrlKey: true, repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    const alert = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('role') === 'alert');
    expect(alert).toBeDefined();
    expect(alert.getAttribute('aria-live')).toBe('assertive');
    expect(alert.textContent).toContain('Ctrl + R');
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0].code).toBe('KeyR');

    const replacement = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-0');
    expect(replacement).not.toBe(capture);
    expect(doc.activeElement).toBe(replacement);
    expect(replacement.value).toBe(t('settings.controls.press_key'));

    let retryPrevented = false;
    let retryStopped = false;
    replacement.dispatch('keydown', {
      code: 'KeyY', key: 'y', repeat: false,
      preventDefault() { retryPrevented = true; },
      stopPropagation() { retryStopped = true; },
    });
    expect(retryPrevented).toBe(true);
    expect(retryStopped).toBe(true);
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings[0].code).toBe('KeyY');
  });

  it('refuses Ctrl+Tab and Ctrl+Escape instead of navigating or closing', () => {
    for (const chord of [
      { code: 'Tab', key: 'Tab', display: 'Ctrl + Tab' },
      { code: 'Escape', key: 'Escape', display: 'Ctrl + Escape' },
    ]) {
      const { doc, registry } = openControls();
      const before = registry.bindingProfile();
      const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
        && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-0');
      capture.dispatch('keydown', {
        code: 'ControlLeft', key: 'Control', ctrlKey: true, repeat: false,
        preventDefault() {}, stopPropagation() {},
      });
      let prevented = false;
      let stopped = false;
      capture.dispatch('keydown', {
        code: chord.code, key: chord.key, ctrlKey: true, repeat: false,
        preventDefault() { prevented = true; },
        stopPropagation() { stopped = true; },
      });
      expect(prevented).toBe(true);
      expect(stopped).toBe(true);
      expect(registry.bindingProfile()).toEqual(before);
      const alert = descendants(bodyOf(doc)).find((el) => el.getAttribute
        && el.getAttribute('role') === 'alert');
      expect(alert.textContent).toContain(chord.display);
      const replacement = descendants(bodyOf(doc)).find((el) => el.getAttribute
        && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-0');
      expect(doc.activeElement).toBe(replacement);
      expect(replacement.value).toBe(t('settings.controls.press_key'));
      expect(findOverlay(doc).hidden).toBe(false);
    }
  });

  it('keeps a standalone modifier through auto-repeat and applies it only on matching keyup', () => {
    const { doc, registry } = openControls();
    const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.view-0');
    capture.dispatch('keydown', {
      code: 'ControlLeft', key: 'Control', ctrlKey: true, repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0].code).toBe('KeyV');

    let repeatPrevented = false;
    let repeatStopped = false;
    capture.dispatch('keydown', {
      code: 'ControlLeft', key: 'Control', ctrlKey: true, repeat: true,
      preventDefault() { repeatPrevented = true; },
      stopPropagation() { repeatStopped = true; },
    });
    expect(repeatPrevented).toBe(false);
    expect(repeatStopped).toBe(false);
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0].code).toBe('KeyV');

    capture.dispatch('keyup', {
      code: 'ControlLeft', key: 'Control', ctrlKey: false,
      preventDefault() {}, stopPropagation() {},
    });
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0]).toMatchObject({
      code: 'ControlLeft', ctrlKey: false,
    });
  });

  it('lets Tab and Escape leave capture untouched and clears a pending modifier', () => {
    for (const navigation of [
      {
        code: 'Tab', key: 'Tab', modifierCode: 'ControlLeft', modifierKey: 'Control',
        modifierFlags: { ctrlKey: true }, navigationFlags: {},
      },
      {
        code: 'Tab', key: 'Tab', modifierCode: 'ControlLeft', modifierKey: 'Control',
        modifierFlags: { ctrlKey: true }, navigationFlags: { shiftKey: true },
      },
      {
        code: 'Escape', key: 'Escape', modifierCode: 'ShiftLeft', modifierKey: 'Shift',
        modifierFlags: { shiftKey: true }, navigationFlags: {},
      },
    ]) {
      const { doc, registry } = openControls();
      const before = registry.bindingProfile();
      const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
        && el.getAttribute('data-control') === 'semantic-binding-captain.red-alert-0');
      capture.dispatch('keydown', {
        code: navigation.modifierCode,
        key: navigation.modifierKey,
        repeat: false,
        ...navigation.modifierFlags,
        preventDefault() {}, stopPropagation() {},
      });

      let prevented = false;
      let stopped = false;
      capture.dispatch('keydown', {
        code: navigation.code,
        key: navigation.key,
        repeat: false,
        ...navigation.navigationFlags,
        preventDefault() { prevented = true; },
        stopPropagation() { stopped = true; },
      });
      expect(prevented).toBe(false);
      expect(stopped).toBe(false);

      // Even if a synthetic harness sends the old modifier's keyup back to
      // this node after navigation, the discarded candidate cannot apply.
      capture.dispatch('keyup', {
        code: navigation.modifierCode,
        key: navigation.modifierKey,
        preventDefault() {}, stopPropagation() {},
      });
      expect(registry.bindingProfile()).toEqual(before);
      expect(descendants(bodyOf(doc))).toContain(capture);
      expect(descendants(bodyOf(doc)).some((el) =>
        el.getAttribute && el.getAttribute('role') === 'alert')).toBe(false);
    }
  });

  it('modal Escape cancels conflict from Reset All and restores the originating capture', () => {
    const { doc, registry } = openControls();
    const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.view-0');
    capture.dispatch('keydown', {
      code: 'KeyR', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    const resetAll = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-reset-all');
    resetAll.focus();
    expect(doc.activeElement).toBe(resetAll);
    let prevented = false;
    let stopped = false;
    findOverlay(doc).dispatch('keydown', {
      target: resetAll,
      code: 'Escape', key: 'Escape',
      preventDefault() { prevented = true; },
      stopPropagation() { stopped = true; },
    });
    expect(prevented).toBe(true);
    expect(stopped).toBe(true);
    expect(findOverlay(doc).hidden).toBe(false);
    expect(descendants(bodyOf(doc)).some((el) =>
      el.className === 'settings-binding-conflict')).toBe(false);
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings[0].code).toBe('KeyV');
    const restored = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.view-0');
    expect(doc.activeElement).toBe(restored);
    expect(restored.value).toBe(t('settings.controls.press_key'));
  });

  it('switching tabs discards hidden conflict state without changing the profile', () => {
    const { doc, registry, inst } = openControls();
    const before = registry.bindingProfile();
    const capture = descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-captain.view-0');
    capture.dispatch('keydown', {
      code: 'KeyR', repeat: false,
      preventDefault() {}, stopPropagation() {},
    });
    expect(descendants(bodyOf(doc)).some((el) =>
      el.className === 'settings-binding-conflict')).toBe(true);
    expect(registry.bindingProfile()).toEqual(before);

    inst.selectTab('audio');
    expect(descendants(bodyOf(doc)).some((el) =>
      el.className === 'settings-binding-conflict')).toBe(false);
    expect(registry.bindingProfile()).toEqual(before);
    inst.selectTab('controls');
    expect(descendants(bodyOf(doc)).some((el) =>
      el.className === 'settings-binding-conflict')).toBe(false);
    expect(registry.bindingProfile()).toEqual(before);

    let prevented = false;
    doc.dispatch('keydown', {
      key: 'Escape', code: 'Escape',
      preventDefault() { prevented = true; },
    });
    expect(prevented).toBe(true);
    expect(findOverlay(doc).hidden).toBe(true);
  });

  it('offers per-action and global reset for both authored slots', () => {
    const { doc, registry } = openControls();
    registry.setBinding(CAPTAIN_RED_ALERT_ACTION_ID, 0, { code: 'KeyY' });
    registry.setBinding(CAPTAIN_RED_ALERT_ACTION_ID, 1, { code: 'KeyU' });
    registry.setBinding(CAPTAIN_VIEW_ACTION_ID, 0, { code: 'KeyJ' });
    registry.setBinding(CAPTAIN_VIEW_ACTION_ID, 1, { code: 'KeyK' });

    descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control')
        === 'semantic-binding-reset-captain.red-alert').click();
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings).toEqual([
      expect.objectContaining({ type: 'keyboard', code: 'KeyR' }),
      { type: 'gamepad', input: 'button', control: 'face-bottom' },
    ]);
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings.map((binding) =>
      binding && binding.code)).toEqual(['KeyJ', 'KeyK']);

    descendants(bodyOf(doc)).find((el) => el.getAttribute
      && el.getAttribute('data-control') === 'semantic-binding-reset-all').click();
    expect(registry.action(CAPTAIN_RED_ALERT_ACTION_ID).bindings).toEqual([
      expect.objectContaining({ type: 'keyboard', code: 'KeyR' }),
      { type: 'gamepad', input: 'button', control: 'face-bottom' },
    ]);
    expect(registry.action(CAPTAIN_VIEW_ACTION_ID).bindings.map((binding) =>
      binding && binding.code)).toEqual(['KeyV', null]);
  });

  it('wires the parent-owned profile to the explicit iframe update seam', () => {
    expect(CLIENT_HTML).toContain('createClientSemanticActionRegistry');
    expect(CLIENT_HTML).toContain('__updateSemanticActionBindings');
    expect(CLIENT_HTML).toContain('pushSemanticBindingsToIframe');
  });
});

describe('accessibility tab', () => {
  const profileState = (accessibilityProfile) => ({
    stations: [{ id: 'helm', holder_token: 'tok1', ratings: ['Std'] }],
    stationRatings: {},
    accessibilityProfile,
  });

  function openAccessibility(accessibilityProfile) {
    const doc = makeDoc();
    const sent = [];
    const writes = [];
    const inst = mount(doc, {
      send: (type, data) => sent.push({ type, data }),
      onAccessibility: (effect, value) => writes.push({ effect, value }),
      getState: () => profileState(accessibilityProfile),
    });
    inst.open();
    inst.selectTab('accessibility');
    return { doc, sent, writes, inst };
  }

  const accSlider = (doc) =>
    bodyOf(doc).children
      .flatMap((s) => s.children)
      .flatMap((c) => (c.children && c.children.length ? c.children : [c]))
      .find((c) => c.type === 'range');

  it('shows a native read failure while keeping explicit choices available', () => {
    const { doc, inst } = openAccessibility();
    doc.defaultView = { PhoenixOsAccessibilityDefaults: {
      availability: { contrast: false, reducedMotion: true, textScale: true },
    } };
    inst.selectTab('audio');
    inst.selectTab('accessibility');
    expect(allText(bodyOf(doc))).toContain(t('settings.accessibility.os_unavailable'));
    expect(bodyButtons(doc).map(button => button.getAttribute('data-control')))
      .toContain('a11y-contrast-on');
  });

  it('appears in the phone tab list and opens a body with effect-named controls', () => {
    const { doc } = openAccessibility();
    expect(tabBarOf(doc).children.map((c) => c.getAttribute('data-tab'))).toContain('accessibility');
    // The load-bearing text-scale slider is present…
    expect(accSlider(doc)).toBeDefined();
    // …alongside the tri-state contrast + motion choice buttons.
    const controls = bodyButtons(doc).map((b) => b.getAttribute('data-control'));
    expect(controls).toContain('a11y-contrast-default');
    expect(controls).toContain('a11y-reducedMotion-on');
  });

  it('sends NOTHING on the wire — every control writes on the client-local path only', () => {
    const { doc, sent, writes } = openAccessibility();
    // Click every button on the tab…
    for (const btn of bodyButtons(doc)) btn.click();
    // …and drag the text-size slider.
    const slider = accSlider(doc);
    slider.value = '1.3';
    slider.dispatch('input');

    // Not one ClientMessage left the panel.
    expect(sent).toEqual([]);
    // But the client-local writes DID happen (persist + apply is client.html's job).
    expect(writes.some((w) => w.effect === 'textScale' && w.value === 1.3)).toBe(true);
    expect(writes.some((w) => w.effect === 'contrast')).toBe(true);
    expect(writes.some((w) => w.effect === 'reducedMotion')).toBe(true);
  });

  it('paints the active tri-state from the stored profile, not from a click', () => {
    const { doc } = openAccessibility({
      presentation: { textScale: 'default', contrast: 'on', reducedMotion: 'off' },
      assistance: {},
    });
    const active = bodyButtons(doc)
      .filter((b) => b.classList.contains('active'))
      .map((b) => b.getAttribute('data-control'));
    expect(active).toContain('a11y-contrast-on');
    expect(active).toContain('a11y-reducedMotion-off');
    expect(active).not.toContain('a11y-contrast-default');
  });

  it('positions the text-size slider from the stored explicit scale', () => {
    const { doc } = openAccessibility({
      presentation: { textScale: 1.4, contrast: 'default', reducedMotion: 'default' },
      assistance: {},
    });
    expect(Number(accSlider(doc).value)).toBeCloseTo(1.4);
  });
});

// ── Master volume ────────────────────────────────────────────────────────────

describe('createMasterVolume', () => {
  it('scales each channel\'s authored level rather than replacing it', () => {
    const quiet = { volume: 0.4 };
    const loud = { volume: 1.0 };
    const master = createMasterVolume([quiet, loud], 0.5);
    expect(quiet.volume).toBeCloseTo(0.2);
    expect(loud.volume).toBeCloseTo(0.5);
    // The authored balance survives every move of the master.
    master.set(0.25);
    expect(quiet.volume).toBeCloseTo(0.1);
    expect(loud.volume).toBeCloseTo(0.25);
  });

  it('is exactly a no-op at 1.0', () => {
    const el = { volume: 0.37 };
    createMasterVolume([el], 1);
    expect(el.volume).toBeCloseTo(0.37);
  });

  it('captures the authored level once, so repeated sets do not compound', () => {
    const el = { volume: 0.8 };
    const master = createMasterVolume([el], 0.5);
    master.set(0.5);
    master.set(0.5);
    expect(el.volume).toBeCloseTo(0.4);
  });

  it('clamps out-of-range and non-numeric masters to the 0..1 scale', () => {
    expect(clampVolume(-1)).toBe(0);
    expect(clampVolume(4)).toBe(1);
    expect(clampVolume('nonsense')).toBe(1);
    expect(clampVolume(0.3)).toBeCloseTo(0.3);
  });

  it('survives a null element in the channel list', () => {
    const el = { volume: 1 };
    expect(() => createMasterVolume([null, el, undefined], 0.5)).not.toThrow();
    expect(el.volume).toBeCloseTo(0.5);
  });

  it('drives the audio element live as the slider moves', () => {
    const doc = makeDoc();
    const el = { volume: 1 };
    const inst = mount(doc, { audioEls: [el] });
    inst.open();
    inst.selectTab('audio');
    const slider = bodyOf(doc).children
      .flatMap((s) => s.children)
      .flatMap((c) => (c.children && c.children.length ? c.children : [c]))
      .find((c) => c.type === 'range');
    expect(slider).toBeDefined();
    slider.value = '0.25';
    // `input`, not `change`: the level has to move under the finger.
    slider.dispatch('input');
    expect(el.volume).toBeCloseTo(0.25);
  });
});
