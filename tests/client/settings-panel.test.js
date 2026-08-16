// Issue #940 — the phone client's settings cog.
//
// The panel is the mirror of the host page's (issue #939): same three tabs,
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
} from '../../gui/settings-panel.js';
import { setBuildFlags, isDemoBuild } from '../../gui/build-flags.js';
import { TABS, CLIENT_DOCUMENTATION_TABS } from '../../gui/settings-tabs.js';
import { ClientSimState } from '../../gui/sim-state.js';

const repoFile = (rel) =>
  fs.readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), '../..', rel),
    'utf-8',
  );

/** client.html itself — the cog's stacking and the build tag are page facts. */
const CLIENT_HTML = repoFile('client.html');
/** The consoles' shared stylesheet, which reserves the cog's corner for it. */
const CONSOLE_CSS = repoFile('gui/console.css');

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
    querySelector() { return null; },
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
    readyState: 'complete',
    createElement(tag) { return makeEl(this, tag); },
    getElementById(id) { return this._byId[id] || null; },
    querySelector(sel) { return this._query[sel] || null; },
    addEventListener() {},
    removeEventListener() {},
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
    expect(labels).toEqual(['audio', 'gameplay', 'debug', 'station-help', 'ship-manual']);
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
    expect(dev).toEqual(TABS.concat(CLIENT_DOCUMENTATION_TABS).map((tb) => tb.id));
    expect(demo).toEqual(TABS.filter((tb) => !tb.gated).concat(CLIENT_DOCUMENTATION_TABS).map((tb) => tb.id));
    expect(dev).toContain('debug');
    expect(demo).not.toContain('debug');
    // Audio and Gameplay are not build-gated — the demo needs both.
    expect(demo).toContain('audio');
    expect(demo).toContain('gameplay');
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
      .toEqual(['audio', 'gameplay', 'station-help', 'ship-manual']);
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
  it('does not add a duplicate fixed station title above the active console', () => {
    expect(CLIENT_HTML).not.toMatch(/id="phase-title"/);
    expect(CLIENT_HTML).not.toMatch(/_consoleTitleEl/);
  });

  it('uses fullscreen and exit glyphs rather than a help glyph', () => {
    expect(CLIENT_HTML).toMatch(/id="fullscreen-btn"[^>]*>⛶<\/button>/);
    expect(CLIENT_HTML).toMatch(/document\.fullscreenElement \? '✕' : '⛶'/);
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

  it('sits top-left, mirroring the host page rather than sharing #status corner', () => {
    const rule = CLIENT_HTML.match(/\.settings-btn\s*\{([^}]*)\}/);
    expect(rule, '.settings-btn rule not found').not.toBeNull();
    expect(rule[1]).toMatch(/top:/);
    expect(rule[1]).not.toMatch(/bottom:/);
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
  // child that escapes its container's padding box, a rule in a file it does
  // not read, or a root font-size other than the browser default (which the
  // first assertion below at least checks nobody has changed). What it does
  // catch is the regression that actually happened here — a gutter tightened
  // back under the cog — and it catches it in every console at once.

  const ROOT_FONT_PX = 16;

  /** The cog's rectangle in CSS px, read from client.html's own declarations. */
  function cogRect() {
    // rem values below only mean 16px while nothing restyles the root.
    expect(
      CLIENT_HTML.match(/(^|[\s>])html\s*\{[^}]*font-size/),
      'client.html restyles the root font-size, so the rem maths below is wrong',
    ).toBeNull();

    const rule = CLIENT_HTML.match(/\.settings-btn\s*\{([^}]*)\}/);
    expect(rule, '.settings-btn rule not found').not.toBeNull();
    const px = (prop) => {
      const m = rule[1].match(new RegExp(`${prop}:\\s*([\\d.]+)(rem|px)`));
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

  // The consoles are iframes filling this page's viewport, so the cog floats
  // over them. `gui/console.css` reserves the corner for all 22 at once, with
  // a compound selector that the per-console `.panel-inner` padding shorthands
  // cannot outrank — asserted here, because a bare `.panel-inner` reservation
  // would be silently overridden by every console that has one.
  it('reserves the cog corner in every console, in both orientations', () => {
    const { right } = cogRect();
    for (const orientation of ['portrait', 'landscape']) {
      const m = CONSOLE_CSS.match(
        new RegExp(
          `@media\\s*\\(orientation:\\s*${orientation}\\)\\s*\\{\\s*` +
            `body\\s+\\.panel-inner\\s*\\{[^}]*padding:\\s*\\d+px\\s+\\d+px\\s+\\d+px\\s+(\\d+)px`,
        ),
      );
      expect(
        m,
        `console.css reserves no ${orientation} compact gutter for the cog ` +
          '(or reserves it with a selector a console can outrank)',
      ).not.toBeNull();
      expect(
        Number(m[1]),
        `the ${orientation} console gutter is under the cog`,
      ).toBeGreaterThanOrEqual(right);
    }
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

  it('sends ToggleQrCode from the QR button', () => {
    openGameplay(withStation);
    const qr = bodyButtons(doc).find((b) => b.textContent === t('settings.toggle_qr'));
    expect(qr).toBeDefined();
    qr.click();
    expect(sent).toEqual([{ type: 'ToggleQrCode', data: {} }]);
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
