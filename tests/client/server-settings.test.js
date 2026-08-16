// @vitest-environment jsdom
//
// Issue #939 — the host page's settings cog.
//
// The cog logic lives in gui/server-settings.js precisely so it can be driven
// here without a browser or a WASM bundle: every simulation call goes through
// an injected `bindings` object, so a fake records what the real page would
// have asked the sim to do. The one thing this file also reads from disk is
// server.html's own #debug-dock markup — the output panel is real page markup
// the module only toggles, so the test drives the module against it rather
// than against a hand-written stand-in that could drift.

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { t } from '../../gui/strings.js';
import {
  mountServerSettings,
  selectOutput,
  visibleTabs,
  DEBUG_OUTPUTS,
} from '../../gui/server-settings.js';
import { isDemoBuild, setBuildFlags, demoFromMeta } from '../../gui/build-flags.js';

const SERVER_HTML = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../server.html',
);
const SRC = fs.readFileSync(SERVER_HTML, 'utf-8');

/** server.html's real #debug-dock subtree, lifted into the test document. */
function installOutputPanel(doc) {
  const parsed = new DOMParser().parseFromString(SRC, 'text/html');
  const dock = parsed.getElementById('debug-dock');
  if (!dock) throw new Error('#debug-dock not found in server.html');
  doc.body.appendChild(doc.importNode(dock, true));
}

/** A stand-in for `window` carrying the bindings server.html publishes. */
function makeBindings(overrides = {}) {
  const calls = [];
  const state = {
    regions: false,
    godmode: false,
    instagib: false,
    paused: false,
    waypoint: true,
    master: 1,
  };
  const record = (name) => (...args) => { calls.push([name, ...args]); };
  const bindings = {
    calls,
    state,
    wasm_toggle_debug_overlay: record('wasm_toggle_debug_overlay'),
    wasm_toggle_debug_damage: record('wasm_toggle_debug_damage'),
    wasm_toggle_debug_entities: record('wasm_toggle_debug_entities'),
    wasm_toggle_entity_inspector: record('wasm_toggle_entity_inspector'),
    wasm_get_debug_state: () => 'MODIFIERS',
    wasm_get_damage_log: () => 'DAMAGE',
    wasm_get_entity_debug_state: () => 'ENTITIES',
    wasm_get_entity_inspector: () => 'INSPECTOR',
    wasm_toggle_debug_regions: () => { calls.push(['wasm_toggle_debug_regions']); state.regions = !state.regions; },
    wasm_is_debug_regions_enabled: () => state.regions,
    wasm_toggle_god_mode: () => { calls.push(['wasm_toggle_god_mode']); state.godmode = !state.godmode; },
    wasm_get_god_mode: () => state.godmode,
    wasm_toggle_instagib: () => { calls.push(['wasm_toggle_instagib']); state.instagib = !state.instagib; },
    wasm_get_instagib: () => state.instagib,
    wasm_teleport_to_waypoint: record('wasm_teleport_to_waypoint'),
    wasm_has_navigation_waypoint: () => state.waypoint,
    wasm_toggle_pause: () => { calls.push(['wasm_toggle_pause']); state.paused = !state.paused; },
    wasm_is_paused: () => state.paused,
    __hostSaveSnapshot: record('__hostSaveSnapshot'),
    __hostResumeSnapshot: record('__hostResumeSnapshot'),
    __hostReturnToLobby: record('__hostReturnToLobby'),
    __hostToggleQrCode: () => { calls.push(['__hostToggleQrCode']); state.qr = !state.qr; },
    __hostIsQrVisible: () => !!state.qr,
    __getMasterVolume: () => state.master,
    __setMasterVolume: (v) => { calls.push(['__setMasterVolume', v]); state.master = v; },
  };
  return Object.assign(bindings, overrides);
}

function mount(opts = {}) {
  const bindings = opts.bindings || makeBindings();
  const menu = mountServerSettings({
    doc: document,
    bindings,
    isDemo: opts.isDemo || (() => false),
    // No rAF loop: every test drives refresh() itself, so nothing keeps
    // running after the assertion.
    autoRefresh: false,
  });
  return { menu, bindings };
}

const $ = (sel) => document.querySelector(sel);
const control = (id) => document.querySelector(`[data-control="${id}"]`);

let mounted = null;

beforeEach(() => {
  document.body.innerHTML = '';
  document.head.innerHTML = '';
  setBuildFlags({ demo: null });
  installOutputPanel(document);
});

afterEach(() => {
  if (mounted) mounted.destroy();
  mounted = null;
  setBuildFlags({ demo: null });
});

// ── Pure helpers ────────────────────────────────────────────────────────────

describe('selectOutput', () => {
  it('turning an output on selects it for viewing and flips only it', () => {
    const next = selectOutput({ enabled: [], viewing: null }, 'damage');
    expect(next.enabled).toEqual(['damage']);
    expect(next.viewing).toBe('damage');
    expect(next.flipped).toBe('damage');
  });

  it('turning off the viewed output falls back to another live one', () => {
    const next = selectOutput({ enabled: ['damage', 'entities'], viewing: 'damage' }, 'damage');
    expect(next.enabled).toEqual(['entities']);
    expect(next.viewing).toBe('entities');
  });

  it('turning off the last output leaves nothing to view', () => {
    const next = selectOutput({ enabled: ['damage'], viewing: 'damage' }, 'damage');
    expect(next.enabled).toEqual([]);
    expect(next.viewing).toBe(null);
  });
});

describe('visibleTabs', () => {
  it('a dev build shows all three tabs, Debug last', () => {
    expect(visibleTabs(false).map((tab) => tab.id)).toEqual(['audio', 'gameplay', 'debug']);
  });

  it('the demo build drops the gated Debug/Cheat tab and keeps the rest', () => {
    expect(visibleTabs(true).map((tab) => tab.id)).toEqual(['audio', 'gameplay']);
  });
});

// ── Cog + tabs (AC1) ────────────────────────────────────────────────────────

describe('the settings cog', () => {
  it('mounts a cog button and an initially closed panel', () => {
    ({ menu: mounted } = mount());
    expect($('#server-settings-btn')).not.toBeNull();
    expect($('#server-settings-overlay').hidden).toBe(true);
    expect(mounted.isOpen()).toBe(false);
  });

  it('clicking the cog opens a panel with the three tabs', () => {
    ({ menu: mounted } = mount());
    $('#server-settings-btn').click();
    expect(mounted.isOpen()).toBe(true);
    const tabs = [...document.querySelectorAll('.server-settings-tab')];
    expect(tabs.map((el) => el.getAttribute('data-tab'))).toEqual(['audio', 'gameplay', 'debug']);
    expect(tabs.map((el) => el.textContent)).toEqual([
      t('settings.tab.audio'),
      t('settings.tab.gameplay'),
      t('settings.tab.debug'),
    ]);
  });

  it('clicking the cog again closes the panel', () => {
    ({ menu: mounted } = mount());
    $('#server-settings-btn').click();
    $('#server-settings-btn').click();
    expect(mounted.isOpen()).toBe(false);
  });
});

// ── Debug toggles + output panel (AC2) ──────────────────────────────────────

describe('the Debug/Cheat tab', () => {
  it('the output panel is hidden until an output is selected', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    const dock = $('#debug-dock');
    expect(dock.classList.contains('open')).toBe(false);

    mounted.open();
    // Debug is now the LAST tab, so the panel opens on Audio; select Debug to
    // exercise its controls (only the active tab's body is built).
    mounted.selectTab('debug');
    expect(dock.classList.contains('open'), 'opening the menu must not open the output panel')
      .toBe(false);
    expect(bindings.calls.length, 'opening the menu must not enable any debug resource')
      .toBe(0);

    control('damage').click();
    expect(dock.classList.contains('open')).toBe(true);
    expect($('#debug-content').textContent).toBe('DAMAGE');
  });

  it('each output flips exactly its own Bevy resource, not all four', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('debug');

    control('entities').click();
    expect(bindings.calls.map((c) => c[0])).toEqual(['wasm_toggle_debug_entities']);

    control('inspector').click();
    expect(bindings.calls.map((c) => c[0])).toEqual([
      'wasm_toggle_debug_entities',
      'wasm_toggle_entity_inspector',
    ]);
  });

  it('deselecting the last output hides the panel and disables that resource', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('debug');
    control('modifiers').click();
    control('modifiers').click();

    expect(bindings.calls.map((c) => c[0])).toEqual([
      'wasm_toggle_debug_overlay',
      'wasm_toggle_debug_overlay',
    ]);
    expect($('#debug-dock').classList.contains('open')).toBe(false);
  });

  it('switching outputs while both are on keeps the panel open on the new stream', () => {
    ({ menu: mounted } = mount());
    mounted.open();
    mounted.selectTab('debug');
    control('modifiers').click();
    control('damage').click();
    expect($('#debug-dock').classList.contains('open')).toBe(true);
    expect($('#debug-content').textContent).toBe('DAMAGE');
  });

  it('every declared output is offered and reads its own stream', () => {
    ({ menu: mounted } = mount());
    mounted.open();
    mounted.selectTab('debug');
    for (const entry of DEBUG_OUTPUTS) {
      expect(control(entry.id), `missing control for ${entry.id}`).not.toBeNull();
    }
  });

  it('cheat toggles drive their bindings and paint from the read-back', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('debug');

    control('godmode').click();
    expect(bindings.state.godmode).toBe(true);
    expect(control('godmode').classList.contains('active')).toBe(true);

    control('wireframes').click();
    expect(bindings.state.regions).toBe(true);
    expect(control('wireframes').classList.contains('active')).toBe(true);

    control('instagib').click();
    expect(bindings.state.instagib).toBe(true);
  });

  it('teleport is disabled while there is no shared Navigation waypoint', () => {
    const bindings = makeBindings();
    bindings.state.waypoint = false;
    ({ menu: mounted } = mount({ bindings }));
    mounted.open();
    mounted.selectTab('debug');

    expect(control('teleport-waypoint').disabled).toBe(true);
    control('teleport-waypoint').click();
    expect(bindings.calls.map((c) => c[0])).not.toContain('wasm_teleport_to_waypoint');

    bindings.state.waypoint = true;
    mounted.refresh();
    expect(control('teleport-waypoint').disabled).toBe(false);
    control('teleport-waypoint').click();
    expect(bindings.calls.map((c) => c[0])).toContain('wasm_teleport_to_waypoint');
  });

  it('save/resume keep the attributes server.html flags outcomes by', () => {
    ({ menu: mounted } = mount());
    mounted.open();
    mounted.selectTab('debug');
    const save = document.querySelector('.debug-action[data-action="save-snapshot"]');
    expect(save, 'flagSnapshotButton() finds the button by this selector').not.toBeNull();
    expect(document.querySelector('.debug-action[data-action="resume-snapshot"]')).not.toBeNull();
  });
});

// ── Audio (AC3) ─────────────────────────────────────────────────────────────

describe('the Audio tab', () => {
  it('the slider applies master volume live, on input', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('audio');

    const slider = $('.server-settings-slider');
    expect(slider.value).toBe('1');
    slider.value = '0.4';
    slider.dispatchEvent(new Event('input'));

    expect(bindings.calls).toContainEqual(['__setMasterVolume', 0.4]);
    expect($('.server-settings-readout').textContent)
      .toBe(t('settings.master_volume_value', { value: '40' }));
  });

  it('the slider opens on the volume the page is already at', () => {
    const bindings = makeBindings();
    bindings.state.master = 0.25;
    ({ menu: mounted } = mount({ bindings }));
    mounted.open();
    mounted.selectTab('audio');
    expect($('.server-settings-slider').value).toBe('0.25');
  });
});

// ── Gameplay (AC4) ──────────────────────────────────────────────────────────

describe('the Gameplay tab', () => {
  it('pause toggles the sim clock and renames itself to resume', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('gameplay');

    expect(control('pause').textContent).toBe(t('settings.gameplay.pause'));
    control('pause').click();
    expect(bindings.state.paused).toBe(true);
    expect(control('pause').textContent).toBe(t('settings.gameplay.resume'));

    control('pause').click();
    expect(bindings.state.paused).toBe(false);
    expect(control('pause').textContent).toBe(t('settings.gameplay.pause'));
  });

  it('exit to lobby asks the host page to return, and closes the menu', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('gameplay');

    control('exit-to-lobby').click();
    expect(bindings.calls.map((c) => c[0])).toContain('__hostReturnToLobby');
    expect(mounted.isOpen()).toBe(false);
  });

  it('toggles the viewscreen join QR from Gameplay and paints its state', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('gameplay');

    expect(control('qr-code').textContent).toBe(t('settings.toggle_qr'));
    expect(control('qr-code').getAttribute('aria-pressed')).toBe('false');

    control('qr-code').click();

    expect(bindings.calls.map((c) => c[0])).toContain('__hostToggleQrCode');
    expect(control('qr-code').getAttribute('aria-pressed')).toBe('true');
  });

  it('replaces the invisible viewscreen hotspot with the Gameplay control', () => {
    expect(SRC).not.toContain('id="qr-toggle-btn"');
    expect(SRC).toContain('window.__hostToggleQrCode');
  });
});

// ── Demo gate (AC5) ──────────────────────────────────────────────────────

describe('the demo build gate', () => {
  it('drops the Debug/Cheat tab entirely, keeping Audio and Gameplay', () => {
    ({ menu: mounted } = mount({ isDemo: () => true }));
    mounted.open();

    const tabs = [...document.querySelectorAll('.server-settings-tab')]
      .map((el) => el.getAttribute('data-tab'));
    expect(tabs).toEqual(['audio', 'gameplay']);
    expect(control('godmode')).toBeNull();
    expect(control('instagib')).toBeNull();
    expect(control('modifiers')).toBeNull();
  });

  it('pause still works in the demo build — it is not debug plumbing', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount({ isDemo: () => true }));
    mounted.open();
    mounted.selectTab('gameplay');
    control('pause').click();
    expect(bindings.calls.map((c) => c[0])).toEqual(['wasm_toggle_pause']);
  });

  it('re-evaluates the gate on every open, since WASM binds late', () => {
    let demo = false;
    ({ menu: mounted } = mount({ isDemo: () => demo }));
    mounted.open();
    // Debug is now the LAST tab, so the panel opens on Audio; select Debug to
    // reach godmode in the dev build.
    mounted.selectTab('debug');
    expect(control('godmode')).not.toBeNull();
    mounted.close();

    demo = true;
    mounted.open();
    // In the demo build Debug is gated away entirely, so godmode is absent no
    // matter which tab is active.
    expect(control('godmode')).toBeNull();
  });
});

describe('isDemoBuild', () => {
  it('defaults to a dev build when nothing says otherwise', () => {
    expect(isDemoBuild({ win: {}, doc: document })).toBe(false);
  });

  it('reads the compiled-in WASM answer (the server page)', () => {
    expect(isDemoBuild({ win: { wasm_is_demo_build: () => true }, doc: document }))
      .toBe(true);
  });

  it('reads a stamped meta tag (server.html <head>, and issue #940)', () => {
    document.head.innerHTML = '<meta name="phoenix-build-demo" content="true">';
    expect(demoFromMeta(document)).toBe(true);
    expect(isDemoBuild({ win: {}, doc: document })).toBe(true);
  });

  // The bug this guards: `wasm_is_demo_build` is bound on
  // TrunkApplicationStarted, but the cog mounts at module evaluation. For the
  // whole WASM download+instantiate window nothing answers the getter, and on
  // the demo build that must NOT read as a dev build.
  it('is the demo build from the tag alone, before WASM has bound anything', () => {
    document.head.innerHTML = '<meta name="phoenix-build-demo" content="true">';
    expect(isDemoBuild({ win: {}, doc: document })).toBe(true);
  });

  // The mirror case: a locally-compiled PHOENIX_DEMO_BUILD carries no stamped
  // tag, because only the demo workflow rewrites the HTML.
  it('is the demo build from the compiled-in flag alone, with a false tag', () => {
    document.head.innerHTML = '<meta name="phoenix-build-demo" content="false">';
    expect(isDemoBuild({ win: { wasm_is_demo_build: () => true }, doc: document }))
      .toBe(true);
  });

  it('is a dev build when the shipped tag is false and WASM says false', () => {
    document.head.innerHTML = '<meta name="phoenix-build-demo" content="false">';
    expect(isDemoBuild({ win: { wasm_is_demo_build: () => false }, doc: document }))
      .toBe(false);
  });

  it('an explicit override wins over both', () => {
    setBuildFlags({ demo: false });
    expect(isDemoBuild({ win: { wasm_is_demo_build: () => true }, doc: document }))
      .toBe(false);
    setBuildFlags({ demo: null });
  });
});

// ── server.html source guards ────────────────────────────────────────
//
// Two behaviours live in server.html's classic scripts, which no module can
// import: the peer-Identify token gate and the unpause-before-exit funnel.
// Neither is reachable from jsdom, so these assert on the shipped source --
// deliberately shape checks, to catch silent removal of a security gate and of
// a fix whose absence is invisible until a host pauses mid-mission.

describe('server.html host-page guards', () => {
  it('ships the demo-build meta tag, defaulted to false', () => {
    expect(SRC).toMatch(/<meta name="phoenix-build-demo" content="false"/);
  });

  it('refuses a peer that identifies under a reserved host-runtime token', () => {
    expect(SRC).toMatch(/function isPeerTokenAllowed\s*\(/);
    expect(SRC).toMatch(/token === LOCAL_CONSOLE_TOKEN\) return false/);
    expect(SRC).toMatch(/token\.startsWith\(AI_TOKEN_PREFIX\)\) return false/);
    // The gate must run before the token is recorded for the connection.
    const identify = SRC.indexOf("msg.type === 'Identify'");
    const gate = SRC.indexOf('isPeerTokenAllowed(claimed)', identify);
    const record = SRC.indexOf('peerTokens.set(conn.peer, token)', identify);
    expect(identify).toBeGreaterThan(-1);
    expect(gate).toBeGreaterThan(identify);
    expect(record).toBeGreaterThan(gate);
  });

  it('unpauses before sending the return, since pause starves FixedUpdate', () => {
    const fn = SRC.indexOf('function hostReturnToLobby()');
    const unpause = SRC.indexOf('window.wasm_toggle_pause()', fn);
    const send = SRC.indexOf("action: 'return_to_lobby'", fn);
    expect(fn).toBeGreaterThan(-1);
    expect(unpause).toBeGreaterThan(fn);
    expect(send).toBeGreaterThan(unpause);
  });

  // The cog is `position: fixed` on <body>, so it only reaches the operator
  // if its z-index clears every full-viewport panel that can be on screen
  // when they want it — not just the one panel a manual check happens to
  // land on. #scenario-panel and .lobby-panel are opaque and cover the
  // whole viewport before a mission starts; that is also the only window
  // where the Audio tab's menu music is playing, so a regression here is a
  // volume control the host cannot reach for the one sound that is audible.
  it('the cog outranks every full-viewport panel it must sit above', () => {
    const zIndexOf = (pattern) => {
      const m = SRC.match(pattern);
      expect(m, `pattern not found in server.html: ${pattern}`).not.toBeNull();
      return Number(m[1]);
    };
    const btnZ = zIndexOf(/#server-settings-btn\s*\{[^}]*z-index:\s*(\d+)/);
    const overlayZ = zIndexOf(/#server-settings-overlay\s*\{[^}]*z-index:\s*(\d+)/);
    const scenarioPanelZ = zIndexOf(/#scenario-panel\s*\{[^}]*z-index:\s*(\d+)/);
    const lobbyPanelZ = zIndexOf(/\.lobby-panel\s*\{[^}]*z-index:\s*(\d+)/);
    const gameOverZ = zIndexOf(/id="game-over-overlay"[^>]*z-index:\s*(\d+)/);

    expect(btnZ).toBeGreaterThan(scenarioPanelZ);
    expect(overlayZ).toBeGreaterThan(scenarioPanelZ);
    expect(btnZ).toBeGreaterThan(lobbyPanelZ);
    expect(overlayZ).toBeGreaterThan(lobbyPanelZ);
    // Already known to hold; guarded so a future restyle can't regress it
    // while "fixing" the panels above.
    expect(btnZ).toBeGreaterThan(gameOverZ);
    expect(overlayZ).toBeGreaterThan(gameOverZ);
  });

  // Winning the z-index fight is what makes the cog *reachable*; it is also
  // what lets it paint over the heading underneath. #world-list-label sat at
  // y 24 under a 34px cog inset 10px from the top, so "SELECT A WORLD" read
  // "⚙ CT A WORLD" on the first screen of every launch and every
  // return-to-lobby. Both panels now reserve the corner instead.
  //
  // HONEST LIMIT: this asserts the *declaration* — that the keep-out token
  // exists, is at least as large as the cog's own extent, and is applied to
  // both panels. It does NOT assert the rendered geometry. jsdom computes no
  // layout, so getBoundingClientRect() here returns zeroes and an overlap
  // check is not expressible in vitest at all. The real geometry guard is the
  // Playwright assertion in tests/smoke/server-settings-cog.spec.js, which
  // measures the actual rects under Chromium; this one only catches someone
  // editing the padding back out.
  it('both top-left panels reserve a keep-out at least as big as the cog', () => {
    const num = (pattern, label) => {
      const m = SRC.match(pattern);
      expect(m, `pattern not found in server.html: ${label}`).not.toBeNull();
      return Number(m[1]);
    };
    // The cog's own extent: inset + size, both authored in px.
    const top = num(/#server-settings-btn\s*\{[^}]*top:\s*(\d+)px/, 'cog top');
    const left = num(/#server-settings-btn\s*\{[^}]*left:\s*(\d+)px/, 'cog left');
    const width = num(/#server-settings-btn\s*\{[^}]*width:\s*(\d+)px/, 'cog width');
    const height = num(/#server-settings-btn\s*\{[^}]*height:\s*(\d+)px/, 'cog height');
    const keepout = num(/--settings-cog-keepout:\s*(\d+)px/, 'keep-out token');

    expect(keepout).toBeGreaterThanOrEqual(left + width);
    expect(keepout).toBeGreaterThanOrEqual(top + height);

    // #world-list: the scenario picker. Its label is the first child, so the
    // reserve has to be the block's own TOP padding.
    const worldList = SRC.match(/#world-list\s*\{([^}]*)\}/);
    expect(worldList, '#world-list rule not found').not.toBeNull();
    expect(worldList[1]).toMatch(/padding:\s*var\(--settings-cog-keepout\)/);

    // .lobby-panel-wrap: #lobby-title is top-left here. The clamp already
    // clears the cog on wide viewports and stops clearing it under ~1100px,
    // so what is guarded is a LEFT floor, not a replacement.
    const lobbyWrap = SRC.match(/\.lobby-panel-wrap\s*\{([^}]*)\}/);
    expect(lobbyWrap, '.lobby-panel-wrap rule not found').not.toBeNull();
    expect(lobbyWrap[1]).toMatch(
      /padding-left:\s*max\([^)]*\([^)]*\),\s*var\(--settings-cog-keepout\)\)/,
    );
  });
});
