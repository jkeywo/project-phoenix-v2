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

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { t } from '../../gui/strings.js';
import {
  mountServerSettings,
  selectOutput,
  reconcileOutputs,
  visibleTabs,
  DEBUG_OUTPUTS,
  DEBUG_TOGGLES,
} from '../../gui/server-settings.js';
import { isDemoBuild, setBuildFlags, demoFromMeta } from '../../gui/build-flags.js';
import { CLIENT_DEBUG_FLAGS } from '../../gui/settings-panel.js';
import {
  ACTION_FEEDBACK_STATE,
  ActionFeedbackLifecycle,
  emitActionFeedbackTransition,
} from '../../gui/action-feedback.js';
import {
  HOST_QR_CODE_ACTION_ID,
  createHostActionRegistry,
} from '../../gui/host-actions.js';
import {
  GM_PAUSE_ACTION_ID,
  GM_RESUME_ACTION_ID,
} from '../../gui/gm-session-actions.js';
import { createGmSessionControls } from '../../gui/gm-session-controls.js';

const SERVER_HTML = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../server.html',
);
const SRC = fs.readFileSync(SERVER_HTML, 'utf-8');

/**
 * The lobby panel's stylesheet, which server.html links rather than inlines
 * since issue #1325 — the native host's lobby document loads the same file.
 * The two cog assertions below reach for `.lobby-panel` rules, so they read
 * them where the rules now live; what they assert about them is unchanged.
 */
const LOBBY_CSS = fs.readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), '../../gui/host-lobby.css'),
  'utf-8',
);

/**
 * The scenario picker's stylesheet, which server.html links rather than
 * inlines since issue #1328 — the native host's lobby document shows the same
 * picker and loads the same file. The two assertions below reach for
 * `#scenario-panel` and `#world-list` rules, so they read them where the rules
 * now live; what they assert about them is unchanged.
 */
const SCENARIOS_CSS = fs.readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), '../../gui/host-scenarios.css'),
  'utf-8',
);

/** The shipped join-code table — the bounds the fleet controls read. */
const JOIN_DATA = JSON.parse(fs.readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), '../../assets/join/join-codes.json'),
  'utf-8',
));

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
    debugFlags: {},
    joinCodeSuffix: 'QUARK',
    codesRotatable: true,
  };
  const record = (name) => (...args) => { calls.push([name, ...args]); };
  const bindings = {
    calls,
    state,
    wasm_set_debug_surface: (surface, on) => {
      calls.push(['wasm_set_debug_surface', surface, on]);
      state.debugFlags[surface] = on;
      if (surface === 'Regions') state.regions = on;
    },
    wasm_get_debug_flags: () => JSON.stringify(state.debugFlags),
    // The four legacy overlays now publish structured JSON the dock parses and
    // renders (issue #1150), not pre-formatted text — so the reads return
    // payloads matching each surface's `codec::encode_*` wire shape.
    wasm_get_debug_state: () =>
      JSON.stringify({ schema_version: 1, flags: [], float_modifiers: [], int_modifiers: [] }),
    wasm_get_damage_log: () =>
      JSON.stringify({
        schema_version: 1,
        entries: [{ source: 'asteroid-9', shield_arc: 'Fore', amount: 4.0 }],
      }),
    wasm_get_entity_debug_state: () =>
      JSON.stringify({
        schema_version: 1,
        entries: [{ name: 'Raider', x: 1, y: 0, z: 2, target: 'none' }],
      }),
    wasm_get_entity_inspector: () =>
      JSON.stringify({ schema_version: 1, player: null, entities: [] }),
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
    // The crew join code + rotate lever (issue #1115).
    __hostJoinCodeState: () => ({ suffix: state.joinCodeSuffix, rotatable: state.codesRotatable }),
    __hostRotateJoinCode: () => {
      calls.push(['__hostRotateJoinCode']);
      if (!state.codesRotatable) return false;
      state.joinCodeSuffix = `${state.joinCodeSuffix}-ROTATED`;
      return true;
    },
    __hostCodesRotatable: () => state.codesRotatable,
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

function pressKey(code, key = code) {
  const event = new KeyboardEvent('keydown', {
    code, key, bubbles: true, cancelable: true,
  });
  document.body.dispatchEvent(event);
  return event;
}

function standardPad(pressed = []) {
  return {
    id: 'test-standard-pad',
    index: 0,
    connected: true,
    mapping: 'standard',
    buttons: Array.from({ length: 16 }, (_, index) => ({
      pressed: pressed.includes(index),
      value: pressed.includes(index) ? 1 : 0,
    })),
    axes: [0, 0, 0, 0],
  };
}

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
  it('a dev build shows all four tabs, Debug last', () => {
    expect(visibleTabs(false).map((tab) => tab.id))
      .toEqual(['audio', 'gameplay', 'controls', 'debug']);
  });

  it('the demo build drops the gated Debug/Cheat tab and keeps the rest', () => {
    expect(visibleTabs(true).map((tab) => tab.id))
      .toEqual(['audio', 'gameplay', 'controls']);
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

  it('clicking the cog opens a panel with the four tabs', () => {
    ({ menu: mounted } = mount());
    $('#server-settings-btn').click();
    expect(mounted.isOpen()).toBe(true);
    const tabs = [...document.querySelectorAll('.server-settings-tab')];
    expect(tabs.map((el) => el.getAttribute('data-tab')))
      .toEqual(['audio', 'gameplay', 'controls', 'debug']);
    expect(tabs.map((el) => el.textContent)).toEqual([
      t('settings.tab.audio'),
      t('settings.tab.gameplay'),
      t('settings.tab.controls'),
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
    // The dock now renders the damage payload as DOM (issue #1150), not text.
    expect($('#debug-content .dbg-damage')).not.toBeNull();
    expect($('#debug-content').textContent).toContain('asteroid-9');
  });

  it('each output flips exactly its own Bevy resource, not all four', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('debug');

    control('entities').click();
    expect(bindings.calls).toEqual([['wasm_set_debug_surface', 'Entities', true]]);

    control('inspector').click();
    expect(bindings.calls).toEqual([
      ['wasm_set_debug_surface', 'Entities', true],
      ['wasm_set_debug_surface', 'Inspector', true],
    ]);
  });

  it('deselecting the last output hides the panel and disables that resource', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('debug');
    control('modifiers').click();
    control('modifiers').click();

    expect(bindings.calls).toEqual([
      ['wasm_set_debug_surface', 'Modifiers', true],
      ['wasm_set_debug_surface', 'Modifiers', false],
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
    // Switching to damage renders its payload; the modifier DOM is gone.
    expect($('#debug-content .dbg-damage')).not.toBeNull();
    expect($('#debug-content').textContent).toContain('asteroid-9');
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

// ── Shared host semantic input + feedback (issue #1281) ────────────────────

describe('the host QR semantic action', () => {
  it('runs the visible control through Pressed, Pending and Applied exactly once', () => {
    const transitions = [];
    window.addEventListener('phoenix-action-feedback', (event) => {
      if (event.detail.actionId === HOST_QR_CODE_ACTION_ID) transitions.push(event.detail);
    });
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('gameplay');

    control('qr-code').click();

    expect(bindings.calls.filter(([name]) => name === '__hostToggleQrCode'))
      .toEqual([['__hostToggleQrCode']]);
    expect(transitions.map(({ state }) => state)).toEqual([
      ACTION_FEEDBACK_STATE.PRESSED,
      ACTION_FEEDBACK_STATE.PENDING,
      ACTION_FEEDBACK_STATE.APPLIED,
    ]);
    expect(new Set(transitions.map(({ correlation }) => correlation)).size).toBe(1);
    expect($('#host-action-feedback').getAttribute('role')).toBe('status');
    expect($('#host-action-feedback').getAttribute('aria-live')).toBe('polite');
    expect($('#host-action-feedback').dataset.state).toBe(ACTION_FEEDBACK_STATE.APPLIED);
    expect($('#host-action-feedback').textContent).toBe(t(
      'semantic_action.host.qr_code.feedback',
      { status: t('action_feedback.applied') },
    ));
  });

  it('keeps the final live result outside the Settings modal', () => {
    ({ menu: mounted } = mount());
    mounted.open();
    mounted.selectTab('gameplay');
    control('qr-code').click();
    mounted.close();

    expect(mounted.isOpen()).toBe(false);
    expect($('#host-action-feedback').textContent).toContain(t('action_feedback.applied'));
    expect($('#host-action-feedback').closest('#server-settings-overlay')).toBeNull();
  });

  it('dispatches the default KeyQ binding while Settings is closed', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    const legacy = vi.fn();
    document.body.addEventListener('keydown', legacy);

    const event = pressKey('KeyQ', 'q');

    expect(event.defaultPrevented).toBe(true);
    expect(legacy).not.toHaveBeenCalled();
    expect(bindings.calls.filter(([name]) => name === '__hostToggleQrCode'))
      .toEqual([['__hostToggleQrCode']]);
    expect(bindings.state.qr).toBe(true);
    expect($('#host-action-feedback').dataset.state).toBe(ACTION_FEEDBACK_STATE.APPLIED);

    mounted.open();
    mounted.selectTab('gameplay');
    expect(control('qr-code').getAttribute('aria-pressed')).toBe('true');
  });

  it('remaps the first slot and uses it with Settings closed', () => {
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('controls');
    const capture = control('semantic-binding-host.qr-code-0');
    capture.focus();
    capture.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyY', key: 'y', bubbles: true, cancelable: true,
    }));
    mounted.close();
    bindings.calls.length = 0;

    expect(pressKey('KeyQ', 'q').defaultPrevented).toBe(false);
    expect(bindings.calls).toEqual([]);
    expect(pressKey('KeyY', 'y').defaultPrevented).toBe(true);
    expect(bindings.calls).toEqual([['__hostToggleQrCode']]);
    expect(bindings.state.qr).toBe(true);
  });

  it('paints persistent on/off and aria-pressed only from host readback', () => {
    const transitions = [];
    window.addEventListener('phoenix-action-feedback', (event) => transitions.push(event.detail));
    let bindings;
    ({ menu: mounted, bindings } = mount());
    mounted.open();
    mounted.selectTab('gameplay');

    bindings.state.qr = true;
    mounted.refresh();
    expect(control('qr-code').getAttribute('aria-pressed')).toBe('true');
    bindings.state.qr = false;
    mounted.refresh();
    expect(control('qr-code').getAttribute('aria-pressed')).toBe('false');
    expect(bindings.calls.filter(([name]) => name === '__hostToggleQrCode')).toEqual([]);
    expect(transitions).toEqual([]);
    expect($('#host-action-feedback').textContent).toBe('');
  });

  it('keeps Controls and its host action in the demo build', () => {
    ({ menu: mounted } = mount({ isDemo: () => true }));
    mounted.open();
    mounted.selectTab('controls');
    expect(control('semantic-binding-host.qr-code-0')).not.toBeNull();
    expect(control('semantic-binding-host.qr-code-1')).not.toBeNull();
    expect(control('godmode')).toBeNull();
  });

  it('shares GM bindings, conflict replacement, keyboard dispatch and gamepad runtime', () => {
    document.body.insertAdjacentHTML('beforeend', `
      <section id="gm-session-controls">
        <h2 id="gm-session-heading"></h2>
        <button id="gm-session-pause"></button>
        <button id="gm-session-resume"></button>
        <p id="gm-session-state"></p>
        <p id="gm-session-feedback"></p>
        <ol id="gm-session-log"></ol>
      </section>
    `);
    const correlations = ['gm-keyboard', 'gm-gamepad'];
    const actionFeedback = new ActionFeedbackLifecycle({
      correlation: () => correlations.shift(),
      onTransition: (value) => emitActionFeedbackTransition(window, value),
    });
    const bindings = makeBindings();
    const hostActions = createHostActionRegistry({
      actionFeedback,
      toggleQrCode: bindings.__hostToggleQrCode,
    });
    const submitSessionPaused = vi.fn(() => true);
    const gmControls = createGmSessionControls({
      doc: document,
      win: window,
      t,
      actions: hostActions,
      actionFeedback,
      submitSessionPaused,
      getOperator: () => ({ id: 'gm-a', name: 'Alex' }),
    });
    let pads = [standardPad()];
    Object.assign(bindings, {
      __hostSemanticActions: hostActions,
      __hostActionFeedback: actionFeedback,
      __hostLocalGm: () => ({ id: 'gm-a', name: 'Alex' }),
      navigator: { getGamepads: () => pads },
    });
    ({ menu: mounted } = mount({ bindings }));
    mounted.gamepad.poll(pads);
    mounted.open();
    mounted.selectTab('controls');

    expect(control(`semantic-binding-${GM_PAUSE_ACTION_ID}-0`)).not.toBeNull();
    expect(control(`semantic-binding-${GM_PAUSE_ACTION_ID}-1`)).not.toBeNull();
    expect(control(`semantic-binding-${GM_RESUME_ACTION_ID}-0`)).not.toBeNull();
    const sharedProfile = mounted.semanticActions.bindingProfile();
    expect(sharedProfile[GM_PAUSE_ACTION_ID]).toHaveLength(2);
    expect(mounted.semanticActions.validateProfile({
      bindings: sharedProfile,
      tuning: mounted.semanticActions.tuningProfile(),
    })).toMatchObject({ status: 'valid' });
    expect(control('semantic-gamepad-select')).not.toBeNull();

    // QR and GM share the active GM context, so the remapper detects rather
    // than silently accepting a competing host-page chord.
    const pauseCapture = control(`semantic-binding-${GM_PAUSE_ACTION_ID}-0`);
    pauseCapture.focus();
    pauseCapture.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyQ', key: 'q', bubbles: true, cancelable: true,
    }));
    expect(control('semantic-binding-conflict-replace')).not.toBeNull();
    control('semantic-binding-conflict-cancel').click();

    mounted.semanticActions.setBinding(GM_PAUSE_ACTION_ID, 0, {
      type: 'keyboard', code: 'KeyY',
    });
    mounted.semanticActions.setBinding(GM_PAUSE_ACTION_ID, 1, {
      type: 'gamepad', input: 'button', control: 'face-bottom',
    });
    mounted.close();
    expect(pressKey('KeyY', 'y').defaultPrevented).toBe(true);
    expect(submitSessionPaused).toHaveBeenNthCalledWith(1, true, 'gm-keyboard');

    mounted.gamepad.select(0);
    mounted.gamepad.poll(pads); // explicit neutral gate
    pads = [standardPad([0])];
    mounted.gamepad.poll(pads);
    expect(submitSessionPaused).toHaveBeenNthCalledWith(2, true, 'gm-gamepad');

    gmControls.destroy();
  });
});

// ── Fleet (issue #1114) ─────────────────────────────────────────────────────

describe('the Fleet section', () => {
  /** Bindings carrying a mutable fleet state the module reads back. */
  function fleetBindings(initial = { open: false }) {
    let fleet = { open: false, owner: false, admission: null, frozen: false, ...initial };
    let role = initial.role === 'gm' ? 'gm' : 'ship';
    const bindings = makeBindings({
      __hostFleetState: () => fleet,
      __hostFleetRole: () => role,
      __hostSetFleetRole: (next) => {
        bindings.calls.push(['__hostSetFleetRole', next]);
        if (!fleet.open && (next === 'ship' || next === 'gm')) role = next;
        return role === next;
      },
      __hostFleetOpen: () => {
        bindings.calls.push(['__hostFleetOpen']);
        fleet = { open: true, owner: true, admission: 'open', frozen: false };
        return true;
      },
      __hostFleetJoin: (code) => {
        bindings.calls.push(['__hostFleetJoin', code]);
        fleet = { open: true, owner: false, admission: 'open', frozen: false };
        return true;
      },
      __hostFleetSetAdmission: (state) => {
        bindings.calls.push(['__hostFleetSetAdmission', state]);
        fleet = { ...fleet, admission: state };
      },
      __hostFleetLeave: () => {
        bindings.calls.push(['__hostFleetLeave']);
        fleet = { open: false, owner: false, admission: null, frozen: false };
      },
      __hostFleetRotate: () => { bindings.calls.push(['__hostFleetRotate']); },
      /** For the tests that need to arrive already frozen. */
      __setFleet: (next) => { fleet = { ...fleet, ...next }; },
    });
    return bindings;
  }

  function openFleetTab(bindings) {
    ({ menu: mounted } = mount({ bindings }));
    mounted.open();
    mounted.selectTab('gameplay');
  }

  const visible = (id) => !!control(id) && control(id).style.display !== 'none';

  it('lives on Gameplay, so the public demo build keeps it', () => {
    ({ menu: mounted } = mount({ bindings: fleetBindings(), isDemo: () => true }));
    mounted.open();
    mounted.selectTab('gameplay');
    expect(control('fleet-open')).toBeTruthy();
  });

  it('offers opening or joining while this host is in no fleet', () => {
    openFleetTab(fleetBindings());
    expect(visible('fleet-open')).toBe(true);
    expect(visible('fleet-code')).toBe(true);
    expect(visible('fleet-join')).toBe(true);
    expect(visible('fleet-leave')).toBe(false);
    expect(visible('fleet-admission')).toBe(false);
  });

  it('defaults to a player ship and lets the privileged host choose GM before joining', () => {
    const bindings = fleetBindings();
    openFleetTab(bindings);
    expect(control('fleet-role-group').getAttribute('role')).toBe('group');
    expect(control('fleet-role-group').getAttribute('aria-label')).toBe(t('settings.fleet.role_heading'));
    expect(control('fleet-role-ship').getAttribute('aria-pressed')).toBe('true');
    expect(control('fleet-role-gm').getAttribute('aria-pressed')).toBe('false');

    control('fleet-role-gm').click();
    expect(bindings.calls).toContainEqual(['__hostSetFleetRole', 'gm']);
    expect(control('fleet-role-ship').getAttribute('aria-pressed')).toBe('false');
    expect(control('fleet-role-gm').getAttribute('aria-pressed')).toBe('true');

    control('fleet-open').click();
    expect(visible('fleet-role-group')).toBe(false);
  });

  it('opening a fleet swaps to the lead\'s controls', () => {
    const bindings = fleetBindings();
    openFleetTab(bindings);
    control('fleet-open').click();

    expect(bindings.calls.map((c) => c[0])).toContain('__hostFleetOpen');
    expect(visible('fleet-open')).toBe(false);
    expect(visible('fleet-admission')).toBe(true);
    expect(visible('fleet-leave')).toBe(true);
    expect(control('fleet-admission').textContent).toBe(t('settings.fleet.close'));
  });

  it('the admission lever closes, renames itself and reopens', () => {
    const bindings = fleetBindings();
    openFleetTab(bindings);
    control('fleet-open').click();

    control('fleet-admission').click();
    expect(bindings.calls).toContainEqual(['__hostFleetSetAdmission', 'closed']);
    expect(control('fleet-admission').textContent).toBe(t('settings.fleet.reopen'));

    control('fleet-admission').click();
    expect(bindings.calls).toContainEqual(['__hostFleetSetAdmission', 'open']);
    expect(control('fleet-admission').textContent).toBe(t('settings.fleet.close'));
  });

  it('disables the lever once the mission froze the roster, rather than hiding it', () => {
    // The operator went looking for this control. A disabled one says "not any
    // more"; one that has vanished says "this menu is broken".
    const bindings = fleetBindings();
    openFleetTab(bindings);
    control('fleet-open').click();
    bindings.__setFleet({ frozen: true, admission: 'closed' });
    mounted.refresh();

    expect(visible('fleet-admission')).toBe(true);
    expect(control('fleet-admission').disabled).toBe(true);
  });

  it('keeps Leave visible but disables it outside the fresh Lobby', () => {
    const bindings = fleetBindings({
      open: true,
      owner: true,
      admission: 'closed',
      frozen: true,
      canLeave: false,
    });
    openFleetTab(bindings);

    expect(visible('fleet-leave')).toBe(true);
    expect(control('fleet-leave').disabled).toBe(true);
    control('fleet-leave').click();
    expect(bindings.calls.map((c) => c[0])).not.toContain('__hostFleetLeave');
  });

  it('joins with the letters typed into the field, and not with an empty one', () => {
    const bindings = fleetBindings();
    openFleetTab(bindings);

    control('fleet-join').click();
    expect(bindings.calls.map((c) => c[0])).not.toContain('__hostFleetJoin');

    control('fleet-code').value = ' quark ';
    control('fleet-join').click();
    expect(bindings.calls).toContainEqual(['__hostFleetJoin', 'quark']);
    // A member gets no admission lever — it is not this host's fleet to close.
    expect(visible('fleet-admission')).toBe(false);
    expect(visible('fleet-leave')).toBe(true);
  });

  it('takes a whole pasted code, sized from the authored bound', () => {
    // The other route through this one field. The fleet panel renders the full
    // structured code as selectable text precisely so the machine being invited
    // — usually a laptop — can paste it, and a length invented here would drop
    // everything past it and then report `malformed`, saying nothing about
    // truncation. `[limits] max_code_length` is the bound the rendezvous
    // service itself applies.
    const authored = JOIN_DATA.limits.max_code_length;
    const bindings = fleetBindings();
    bindings.__hostFleetCodeLimit = () => authored;
    openFleetTab(bindings);
    expect(control('fleet-code').maxLength).toBe(authored);

    const full = [
      JOIN_DATA.namespaces.server, JOIN_DATA.version.guid, 'QUARK',
    ].join('_');
    expect(full.length).toBeGreaterThan(32);
    expect(full.length).toBeLessThanOrEqual(authored);
    control('fleet-code').value = full;
    control('fleet-join').click();
    expect(bindings.calls).toContainEqual(['__hostFleetJoin', full]);
  });

  it('leaves the field unbounded until the host page knows the authored bound', () => {
    // A page mid-boot has not fetched the join table yet. Unbounded is the
    // honest state: accepting too much is a refusal the operator can read,
    // accepting too little is a silent lie about what they typed.
    openFleetTab(fleetBindings());
    expect(control('fleet-code').getAttribute('maxlength')).toBeNull();
  });

  it('leaving puts the open/join pair back', () => {
    const bindings = fleetBindings();
    openFleetTab(bindings);
    control('fleet-open').click();
    control('fleet-leave').click();
    expect(bindings.calls.map((c) => c[0])).toContain('__hostFleetLeave');
    expect(visible('fleet-open')).toBe(true);
    expect(visible('fleet-leave')).toBe(false);
  });

  it('degrades to no fleet at all when the host page publishes no bindings', () => {
    // Every other control on this tab survives a missing binding; this one has
    // to as well, or a page mid-boot renders a broken menu.
    ({ menu: mounted } = mount());
    mounted.open();
    mounted.selectTab('gameplay');
    expect(visible('fleet-open')).toBe(true);
    expect(visible('fleet-admission')).toBe(false);
    expect(() => mounted.refresh()).not.toThrow();
  });

  it('is wired to bindings server.html actually publishes', () => {
    for (const name of [
      '__hostFleetState', '__hostFleetOpen', '__hostFleetJoin',
      '__hostFleetSetAdmission', '__hostFleetLeave', '__hostFleetCodeLimit',
      '__hostFleetRotate', '__hostFleetRole', '__hostSetFleetRole',
    ]) {
      expect(SRC, name).toContain(`window.${name}`);
    }
  });

  it('threads the selected role and private GM reconnect capability through the host-only mesh seam', () => {
    expect(SRC).toContain('role: fleetRole');
    expect(SRC).toContain('reconnectCredential: priorIdentity ? priorIdentity.reconnectCredential : null');
    expect(SRC).toContain('onIdentity: (identity) =>');
    expect(SRC).toContain('rememberGmIdentity(code, identity)');
    expect(SRC).toContain('window.wasm_set_gm_roster(JSON.stringify(gms))');
    expect(SRC).not.toContain('ClientMessage::SetFleetRole');
  });

  // ── Rotating the fleet code (issue #1115) ─────────────────────────────────

  it('offers the rotate lever only to the fleet\'s own lead', () => {
    const bindings = fleetBindings();
    bindings.__hostCodesRotatable = () => true;
    openFleetTab(bindings);
    control('fleet-open').click();
    expect(control('fleet-rotate')).toBeTruthy();
    expect(visible('fleet-rotate')).toBe(true);

    // A member has no code of its own to rotate — it is a guest on the
    // lead's — so the lever stays hidden for it.
    control('fleet-leave').click();
    control('fleet-code').value = 'quark';
    control('fleet-join').click();
    expect(visible('fleet-rotate')).toBe(false);
  });

  it('rotating calls the binding', () => {
    const bindings = fleetBindings();
    bindings.__hostCodesRotatable = () => true;
    openFleetTab(bindings);
    control('fleet-open').click();
    control('fleet-rotate').click();
    expect(bindings.calls.map((c) => c[0])).toContain('__hostFleetRotate');
  });

  it('disables the rotate lever, with a tooltip, while a mission is running — independently of the freeze latch', () => {
    const bindings = fleetBindings();
    bindings.__hostCodesRotatable = () => false;
    openFleetTab(bindings);
    control('fleet-open').click();
    // Not frozen — the mission-phase gate is what disables it here, exactly
    // as it does for the crew code's own lever.
    expect(control('fleet-admission').disabled).toBe(false);
    expect(control('fleet-rotate').disabled).toBe(true);
    expect(control('fleet-rotate').classList.contains('disabled')).toBe(true);
    expect(control('fleet-rotate').title).toBe(t('settings.gameplay.rotate_locked_hint'));
  });
});

// ── Join code rotation, crew and fleet (issue #1115) ────────────────────────

describe('the Join Code section', () => {
  function openGameplayTab(bindings) {
    ({ menu: mounted } = mount({ bindings }));
    mounted.open();
    mounted.selectTab('gameplay');
  }

  it('shows the current crew code and a rotate lever', () => {
    openGameplayTab(makeBindings());
    expect(control('join-code-readout').textContent).toBe('QUARK');
    expect(control('rotate-join-code')).toBeTruthy();
    expect(control('rotate-join-code').disabled).toBe(false);
  });

  it('rotating calls the binding and repaints the new suffix', () => {
    const bindings = makeBindings();
    openGameplayTab(bindings);
    control('rotate-join-code').click();
    expect(bindings.calls).toContainEqual(['__hostRotateJoinCode']);
    expect(control('join-code-readout').textContent).toBe('QUARK-ROTATED');
  });

  it('disables the lever, with a tooltip, while a mission is running (AC2)', () => {
    const bindings = makeBindings();
    bindings.state.codesRotatable = false;
    openGameplayTab(bindings);
    const rotate = control('rotate-join-code');
    expect(rotate.disabled).toBe(true);
    expect(rotate.classList.contains('disabled')).toBe(true);
    expect(rotate.title).toBe(t('settings.gameplay.rotate_locked_hint'));
  });

  it('degrades to a blank readout and a disabled lever with no bindings published', () => {
    ({ menu: mounted } = mount({ bindings: {} }));
    mounted.open();
    mounted.selectTab('gameplay');
    expect(control('join-code-readout').textContent).toBe('');
    expect(control('rotate-join-code').disabled).toBe(true);
    expect(() => mounted.refresh()).not.toThrow();
  });

  it('is wired to bindings server.html actually publishes', () => {
    for (const name of ['__hostJoinCodeState', '__hostRotateJoinCode', '__hostCodesRotatable']) {
      expect(SRC, name).toContain(`window.${name}`);
    }
  });
});


// ── Demo gate (AC5) ──────────────────────────────────────────────────────

describe('the demo build gate', () => {
  it('drops Debug/Cheat while keeping Audio, Gameplay and Controls', () => {
    ({ menu: mounted } = mount({ isDemo: () => true }));
    mounted.open();

    const tabs = [...document.querySelectorAll('.server-settings-tab')]
      .map((el) => el.getAttribute('data-tab'));
    expect(tabs).toEqual(['audio', 'gameplay', 'controls']);
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

  it('routes the debug_regions URL option through the one catalogue mutator', () => {
    expect(SRC).not.toContain('wasm_set_debug_regions');
    expect(SRC).not.toContain('wasm_is_debug_regions_enabled');
    const loadAt = SRC.indexOf("const _debugSurfacesReady = import('./gui/debug-surfaces.generated.js')");
    const startAt = SRC.indexOf('async function startServer()');
    const awaitAt = SRC.indexOf('await _debugSurfacesReady;', startAt);
    const setAt = SRC.indexOf(
      'wasm_set_debug_surface(window.phDebugSurfaces.DebugSurface.Regions, true)',
      startAt,
    );
    expect(loadAt).toBeGreaterThanOrEqual(0);
    expect(loadAt).toBeLessThan(startAt);
    expect(awaitAt).toBeGreaterThan(startAt);
    expect(setAt).toBeGreaterThan(awaitAt);
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
    const zIndexIn = (source, label) => (pattern) => {
      const m = source.match(pattern);
      expect(m, `pattern not found in ${label}: ${pattern}`).not.toBeNull();
      return Number(m[1]);
    };
    const zIndexOf = zIndexIn(SRC, 'server.html');
    const zIndexOfLobby = zIndexIn(LOBBY_CSS, 'gui/host-lobby.css');
    const zIndexOfScenarios = zIndexIn(SCENARIOS_CSS, 'gui/host-scenarios.css');
    const btnZ = zIndexOf(/#server-settings-btn\s*\{[^}]*z-index:\s*(\d+)/);
    const overlayZ = zIndexOf(/#server-settings-overlay\s*\{[^}]*z-index:\s*(\d+)/);
    // Lives in gui/host-scenarios.css since #1328; the cog it must sit under is
    // still this page's.
    const scenarioPanelZ = zIndexOfScenarios(/#scenario-panel\s*\{[^}]*z-index:\s*(\d+)/);
    const lobbyPanelZ = zIndexOfLobby(/\.lobby-panel\s*\{[^}]*z-index:\s*(\d+)/);
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
    // reserve has to be the block's own TOP padding. The rule lives in
    // gui/host-scenarios.css since #1328; the token it names is still this
    // page's, and the optional fallback is that sheet saying the token may be
    // absent — it is, on the native lobby surface, which has no cog.
    const worldList = SCENARIOS_CSS.match(/#world-list\s*\{([^}]*)\}/);
    expect(worldList, '#world-list rule not found').not.toBeNull();
    expect(worldList[1]).toMatch(/padding:\s*var\(--settings-cog-keepout(?:,[^)]*)?\)/);

    // .lobby-panel-wrap: #lobby-title is top-left here. The clamp already
    // clears the cog on wide viewports and stops clearing it under ~1100px,
    // so what is guarded is a LEFT floor, not a replacement. The rule lives in
    // gui/host-lobby.css since #1325; the token it names is still this page's.
    const lobbyWrap = LOBBY_CSS.match(/\.lobby-panel-wrap\s*\{([^}]*)\}/);
    expect(lobbyWrap, '.lobby-panel-wrap rule not found').not.toBeNull();
    // The optional fallback is the sheet saying the token may be absent — it is,
    // on the native lobby surface, which has no cog. What is guarded is that the
    // host page's floor still names the token.
    expect(lobbyWrap[1]).toMatch(
      /padding-left:\s*max\([^)]*\([^)]*\),\s*var\(--settings-cog-keepout(?:,[^)]*)?\)\)/,
    );
  });
});

// ── Following the simulation's own flags (issue #1169 review, finding C2) ────
//
// The debug outputs had no read-back, so this module painted from its memory of
// what it had last clicked. That memory is not authoritative: a connected PHONE
// can flip the same flags. For a render-only overlay a stale highlight is
// cosmetic; for console latency, which gates MEASUREMENT, it meant a cog reading
// "on" over a simulation measuring nothing and dropping every batch a phone sent.

describe('reconcileOutputs', () => {
  const OUTPUTS = [
    { id: 'console-latency', flag: 'ConsoleLatency' },
    { id: 'station-activity', flag: 'StationActivity' },
  ];

  it('turns an output ON when the simulation says it is', () => {
    const next = reconcileOutputs(
      { enabled: [], viewing: null },
      { ConsoleLatency: true },
      OUTPUTS,
    );
    expect(next.enabled).toEqual(['console-latency']);
  });

  it('turns an output OFF when a phone turned it off behind the cog', () => {
    const next = reconcileOutputs(
      { enabled: ['console-latency'], viewing: 'console-latency' },
      { ConsoleLatency: false },
      OUTPUTS,
    );
    expect(next.enabled).toEqual([]);
    expect(next.viewing).toBeNull();
  });

  it('falls back to another live output rather than showing a dead panel', () => {
    const next = reconcileOutputs(
      { enabled: ['station-activity', 'console-latency'], viewing: 'console-latency' },
      { ConsoleLatency: false },
      OUTPUTS,
    );
    expect(next.viewing).toBe('station-activity');
  });

  it('leaves an output unchanged when its catalogue row is absent from a partial report', () => {
    const state = { enabled: ['station-activity'], viewing: 'station-activity' };
    const next = reconcileOutputs(state, { ConsoleLatency: true }, OUTPUTS);
    expect(next.enabled).toContain('station-activity');
  });

  it('never guesses: no report from the simulation changes nothing', () => {
    const state = { enabled: ['console-latency'], viewing: 'console-latency' };
    expect(reconcileOutputs(state, null, OUTPUTS)).toBe(state);
    expect(reconcileOutputs(state, {}, OUTPUTS).enabled).toEqual(['console-latency']);
  });

  it('is idempotent, since it runs on every paint', () => {
    const once = reconcileOutputs({ enabled: [], viewing: null }, { ConsoleLatency: true }, OUTPUTS);
    const twice = reconcileOutputs(once, { ConsoleLatency: true }, OUTPUTS);
    expect(twice.enabled).toEqual(once.enabled);
  });
});

describe('catalogue-backed outputs — absolute set, not host-local toggles', () => {
  it('host and phone expose the same one-to-one Debug Surface set', () => {
    const hostFlags = [...DEBUG_OUTPUTS, ...DEBUG_TOGGLES]
      .map((entry) => entry.flag)
      .filter(Boolean);
    const phoneFlags = CLIENT_DEBUG_FLAGS.map((entry) => entry.flag);
    expect(new Set(hostFlags).size).toBe(hostFlags.length);
    expect(new Set(phoneFlags).size).toBe(phoneFlags.length);
    expect(hostFlags.slice().sort()).toEqual(phoneFlags.slice().sort());
  });

  it('declares a unique catalogue flag for every output', () => {
    const flags = DEBUG_OUTPUTS.map((entry) => entry.flag);
    expect(flags).not.toContain(undefined);
    expect(new Set(flags).size).toBe(flags.length);
  });

  /** Open the cog on the Debug tab with a scripted flag read-back. */
  function cogWithFlags(flags) {
    const sets = [];
    const bindings = makeBindings({
      wasm_get_debug_flags: () => JSON.stringify(flags),
      wasm_set_debug_surface: (surface, on) => {
        sets.push([surface, on]);
        flags[surface] = on;
      },
    });
    ({ menu: mounted } = mount({ bindings }));
    mounted.open();
    mounted.selectTab('debug');
    return sets;
  }

  it('sends the value it wants, computed from the simulation read-back', () => {
    const sets = cogWithFlags({ ConsoleLatency: false });
    control('console-latency').click();
    expect(sets).toEqual([['ConsoleLatency', true]]);
  });

  it('a phone that turned it on behind the cog makes the next click turn it OFF', () => {
    // The simulation says ON even though this cog never clicked it.
    const sets = cogWithFlags({ ConsoleLatency: true });
    control('console-latency').click();
    // A relative toggle would have turned it on AGAIN — silently leaving the
    // simulation measuring nothing while the button read "on".
    expect(sets).toEqual([['ConsoleLatency', false]]);
  });

  it('paints the button from the simulation, not from what it last clicked', () => {
    cogWithFlags({ ConsoleLatency: true });
    expect(control('console-latency').getAttribute('aria-pressed')).toBe('true');
  });
});
