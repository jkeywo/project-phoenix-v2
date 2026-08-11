/**
 * gui/server-settings.js — the host page's settings cog (issue #939).
 *
 * A gear in the top-left of server.html opens a three-tab panel:
 *
 *   - **Debug / Cheat** — the toggles that used to be loose buttons in the
 *     debug dock's toolbar, plus the four debug OUTPUT selectors. Absent
 *     entirely in the public demo build (see `isDemo` below).
 *   - **Audio** — master volume, which SCALES the per-sound volumes authored
 *     in the ship/world TOML rather than replacing them.
 *   - **Gameplay** — pause/resume, the viewscreen join QR, and exit-to-lobby.
 *     Deliberately NOT build-gated, so nothing on this tab may reach for
 *     debug-only plumbing.
 *
 * Two behaviours are new rather than moved:
 *
 *   1. **Per-toggle debug resources.** Opening the old dock force-enabled all
 *      four debug Bevy resources at once and closing disabled all four. Each
 *      output now flips exactly its own resource, which is what makes "the
 *      output panel is hidden until you select an output" mean anything.
 *   2. **The output panel pops out.** `#debug-dock` is hidden until an output
 *      is selected and hides again when the last one is deselected.
 *
 * Everything that reaches the simulation goes through `bindings` — the
 * `window` object in the page, an injected stub under vitest. No WASM symbol
 * is referenced by anything except the tables below, so the whole module runs
 * headless in jsdom.
 */

import { t } from './strings.js';
import { isDemoBuild } from './build-flags.js';
import { TABS, visibleTabs, resolveActiveTab } from './settings-tabs.js';

// ── Wiring tables ────────────────────────────────────────────────────────────
//
// One row per control: its string id, the binding it calls, and (where one
// exists) the binding that reports its current state. Nothing else in this
// file spells a `wasm_*` name.

/**
 * Debug overlays that also produce a text stream for the output panel. `read`
 * is polled while that output is the visible one; `toggle` flips the matching
 * Bevy resource (`DebugOverlayEnabled`, `DebugDamageEnabled`,
 * `DebugEntitiesEnabled`, `DebugEntityInspectorEnabled`).
 */
export const DEBUG_OUTPUTS = [
  { id: 'modifiers', labelId: 'settings.debug.modifiers', toggle: 'wasm_toggle_debug_overlay', read: 'wasm_get_debug_state' },
  { id: 'damage', labelId: 'settings.debug.damage', toggle: 'wasm_toggle_debug_damage', read: 'wasm_get_damage_log' },
  { id: 'entities', labelId: 'settings.debug.entities', toggle: 'wasm_toggle_debug_entities', read: 'wasm_get_entity_debug_state' },
  { id: 'inspector', labelId: 'settings.debug.inspector', toggle: 'wasm_toggle_entity_inspector', read: 'wasm_get_entity_inspector' },
];

/** Cheats and world-drawing toggles, each with an authoritative read-back. */
export const DEBUG_TOGGLES = [
  { id: 'wireframes', labelId: 'settings.debug.wireframes', toggle: 'wasm_toggle_debug_regions', state: 'wasm_is_debug_regions_enabled' },
  { id: 'godmode', labelId: 'settings.debug.godmode', toggle: 'wasm_toggle_god_mode', state: 'wasm_get_god_mode' },
  { id: 'instagib', labelId: 'settings.debug.instagib', toggle: 'wasm_toggle_instagib', state: 'wasm_get_instagib' },
];

/**
 * One-shot debug commands. `save-snapshot` / `resume-snapshot` go through host
 * shims rather than the raw exports because a save needs the page's slot id
 * and a resume is a page reload, neither of which is this module's business.
 * `enabled` gates the control when the simulation says the action is possible.
 */
export const DEBUG_COMMANDS = [
  { id: 'teleport-waypoint', labelId: 'settings.debug.teleport', call: 'wasm_teleport_to_waypoint', enabled: 'wasm_has_navigation_waypoint' },
  { id: 'save-snapshot', labelId: 'settings.debug.save_snapshot', call: '__hostSaveSnapshot' },
  { id: 'resume-snapshot', labelId: 'settings.debug.resume_snapshot', call: '__hostResumeSnapshot' },
];

// The tab list — and the "which tab survives this build" answer — moved to
// `gui/settings-tabs.js` when the phone client grew the same three tabs (issue
// #940). Both pages must gate the same tab in the same build, so both import
// from there; this page keeps no copy of its own. Re-exported here so this
// module's existing importers are unchanged.
export { TABS, visibleTabs, resolveActiveTab };

const BUTTON_ID = 'server-settings-btn';
const OVERLAY_ID = 'server-settings-overlay';
const OUTPUT_HOST_ID = 'debug-dock';
const OUTPUT_CONTENT_ID = 'debug-content';

// The slider's own resolution, not a tunable: master volume is a 0..1 scale
// factor and the UI offers it in whole percent.
const VOLUME_MIN = 0;
const VOLUME_MAX = 1;
const VOLUME_STEP = 0.01;

/**
 * Fold a click on debug output `id` into the next {enabled, viewing} state.
 *
 * Pure, so the awkward part — what the output panel shows when you turn OFF
 * the output you were looking at while another is still enabled — is tested
 * without a DOM. Returns the new state plus the single output whose Bevy
 * resource must be flipped.
 *
 * @param {{ enabled: string[], viewing: string|null }} state
 * @param {string} id
 * @returns {{ enabled: string[], viewing: string|null, flipped: string }}
 */
export function selectOutput(state, id) {
  const enabled = state.enabled.slice();
  const at = enabled.indexOf(id);
  if (at >= 0) {
    enabled.splice(at, 1);
    // Turning off what you were watching falls back to another live output
    // rather than leaving an empty panel open.
    const viewing = state.viewing === id ? (enabled.length > 0 ? enabled[0] : null) : state.viewing;
    return { enabled, viewing, flipped: id };
  }
  enabled.push(id);
  return { enabled, viewing: id, flipped: id };
}

// ── Mount ────────────────────────────────────────────────────────────────────

/**
 * Mount the cog, its panel, and the output-panel wiring on `doc`.
 *
 * @param {{
 *   doc?: Document,
 *   bindings?: object,   // defaults to `window`
 *   isDemo?: () => boolean,
 *   autoRefresh?: boolean,
 * }} opts
 * @returns {{ open: function, close: function, isOpen: function,
 *             refresh: function, selectTab: function, destroy: function }}
 */
export function mountServerSettings(opts = {}) {
  const doc = opts.doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) {
    return {
      open() {}, close() {}, isOpen: () => false,
      refresh() {}, selectTab() {}, destroy() {},
    };
  }
  const win = doc.defaultView || (typeof window !== 'undefined' ? window : null);
  const bindings = opts.bindings || win || {};
  const isDemo = opts.isDemo || (() => isDemoBuild({ win: bindings, doc }));
  const autoRefresh = opts.autoRefresh !== false;

  /** Call binding `name` if the page has published it; else undefined. */
  const invoke = (name, ...args) => {
    const fn = name ? bindings[name] : null;
    if (typeof fn !== 'function') return undefined;
    try {
      return fn(...args);
    } catch (e) {
      console.warn('[Phoenix] settings binding failed:', name, e);
      return undefined;
    }
  };

  // Debug output state — module-owned because the four resources have no
  // read-back exports; this module is their only caller.
  let outputs = { enabled: [], viewing: null };
  let activeTab = null;
  let rafHandle = null;
  const controls = {
    toggles: {}, commands: {}, outputs: {}, pause: null, qr: null,
    volumeReadout: null,
  };

  // ── Elements ───────────────────────────────────────────────────────────────

  let btn = doc.getElementById(BUTTON_ID);
  if (!btn) {
    btn = doc.createElement('button');
    btn.id = BUTTON_ID;
    btn.className = 'server-settings-btn';
    btn.type = 'button';
    doc.body.appendChild(btn);
  }
  btn.textContent = '⚙';
  btn.title = t('settings.title');
  btn.setAttribute('aria-label', t('settings.title'));
  btn.setAttribute('aria-expanded', 'false');

  let overlay = doc.getElementById(OVERLAY_ID);
  if (!overlay) {
    overlay = doc.createElement('div');
    overlay.id = OVERLAY_ID;
    overlay.className = 'server-settings-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    doc.body.appendChild(overlay);
  }
  overlay.hidden = true;
  overlay.setAttribute('aria-hidden', 'true');

  const outputHost = doc.getElementById(OUTPUT_HOST_ID);
  const outputContent = doc.getElementById(OUTPUT_CONTENT_ID);

  // ── Small builders ─────────────────────────────────────────────────────────

  function section(labelId) {
    const el = doc.createElement('div');
    el.className = 'server-settings-section';
    const heading = doc.createElement('div');
    heading.className = 'server-settings-heading';
    heading.textContent = t(labelId);
    el.appendChild(heading);
    return el;
  }

  function hint(labelId) {
    const el = doc.createElement('div');
    el.className = 'server-settings-hint';
    el.textContent = t(labelId);
    return el;
  }

  function rowHost() {
    const el = doc.createElement('div');
    el.className = 'server-settings-row';
    return el;
  }

  function control(id, labelId, onClick) {
    const el = doc.createElement('button');
    el.type = 'button';
    el.className = 'server-settings-control';
    el.setAttribute('data-control', id);
    el.textContent = t(labelId);
    el.setAttribute('aria-pressed', 'false');
    el.addEventListener('click', (e) => {
      if (e && typeof e.preventDefault === 'function') e.preventDefault();
      if (el.disabled) return;
      onClick();
    });
    return el;
  }

  // ── Output panel ───────────────────────────────────────────────────────────

  /** Show/hide `#debug-dock` and paint the visible stream. */
  function paintOutput() {
    const viewing = outputs.viewing;
    if (outputHost) {
      if (viewing) outputHost.classList.add('open');
      else outputHost.classList.remove('open');
      outputHost.setAttribute('aria-hidden', viewing ? 'false' : 'true');
    }
    if (outputContent && viewing) {
      const entry = DEBUG_OUTPUTS.find((o) => o.id === viewing);
      const text = entry ? invoke(entry.read) : undefined;
      outputContent.textContent = typeof text === 'string' ? text : '';
    }
    for (const entry of DEBUG_OUTPUTS) {
      const el = controls.outputs[entry.id];
      if (!el) continue;
      const on = outputs.enabled.indexOf(entry.id) >= 0;
      el.classList.toggle('active', on);
      el.classList.toggle('viewing', outputs.viewing === entry.id);
      el.setAttribute('aria-pressed', on ? 'true' : 'false');
    }
  }

  function clickOutput(id) {
    const next = selectOutput(outputs, id);
    // Exactly one resource flips per click — the per-toggle behaviour that
    // replaced the old "opening the dock enables all four" bundle.
    const entry = DEBUG_OUTPUTS.find((o) => o.id === next.flipped);
    if (entry) invoke(entry.toggle);
    outputs = { enabled: next.enabled, viewing: next.viewing };
    paintOutput();
  }

  // ── Tab bodies ─────────────────────────────────────────────────────────────

  function buildDebugTab(body) {
    const outputSection = section('settings.debug.output');
    outputSection.appendChild(hint('settings.debug.output_hint'));
    const outputRow = rowHost();
    for (const entry of DEBUG_OUTPUTS) {
      const el = control(entry.id, entry.labelId, () => clickOutput(entry.id));
      controls.outputs[entry.id] = el;
      outputRow.appendChild(el);
    }
    outputSection.appendChild(outputRow);
    body.appendChild(outputSection);

    const cheatSection = section('settings.debug.cheats');
    const cheatRow = rowHost();
    for (const entry of DEBUG_TOGGLES) {
      const el = control(entry.id, entry.labelId, () => {
        invoke(entry.toggle);
        // God mode crosses command admission and lands a tick later, so every
        // toggle's active class is painted from the read-back in refresh()
        // rather than assumed here (issue #900).
        refresh();
      });
      controls.toggles[entry.id] = el;
      cheatRow.appendChild(el);
    }
    cheatSection.appendChild(cheatRow);
    body.appendChild(cheatSection);

    const sessionSection = section('settings.debug.session');
    const sessionRow = rowHost();
    for (const entry of DEBUG_COMMANDS) {
      const el = control(entry.id, entry.labelId, () => invoke(entry.call));
      // The snapshot outcome is flagged on the button by server.html's
      // `flagSnapshotButton`, which finds it by this attribute pair.
      el.classList.add('debug-action');
      el.setAttribute('data-action', entry.id);
      controls.commands[entry.id] = el;
      sessionRow.appendChild(el);
    }
    sessionSection.appendChild(sessionRow);
    body.appendChild(sessionSection);
  }

  function buildAudioTab(body) {
    const el = section('settings.master_volume');
    const row = rowHost();

    const slider = doc.createElement('input');
    slider.type = 'range';
    slider.className = 'server-settings-slider';
    slider.min = String(VOLUME_MIN);
    slider.max = String(VOLUME_MAX);
    slider.step = String(VOLUME_STEP);
    const current = invoke('__getMasterVolume');
    slider.value = String(typeof current === 'number' ? current : VOLUME_MAX);

    const readout = doc.createElement('span');
    readout.className = 'server-settings-readout';
    controls.volumeReadout = readout;

    const paintReadout = () => {
      readout.textContent = t('settings.master_volume_value', {
        value: String(Math.round(Number(slider.value) * 100)),
      });
    };
    // `input` applies live while dragging — the acceptance criterion is that
    // you hear the change as you move the slider, not on release.
    const apply = () => {
      invoke('__setMasterVolume', Number(slider.value));
      paintReadout();
    };
    slider.addEventListener('input', apply);
    slider.addEventListener('change', apply);
    paintReadout();

    row.appendChild(slider);
    row.appendChild(readout);
    el.appendChild(row);
    el.appendChild(hint('settings.master_volume_hint'));
    body.appendChild(el);
  }

  function buildGameplayTab(body) {
    const sim = section('settings.gameplay.simulation');
    const simRow = rowHost();
    // Pause is a GAMEPLAY control: it reaches `wasm_toggle_pause`, which is
    // compiled into every build, and it is built here whether or not the
    // Debug/Cheat tab exists.
    const pause = control('pause', 'settings.gameplay.pause', () => {
      invoke('wasm_toggle_pause');
      refresh();
    });
    controls.pause = pause;
    simRow.appendChild(pause);
    sim.appendChild(simRow);
    body.appendChild(sim);

    const qrSection = section('settings.qr_code');
    const qrRow = rowHost();
    const qr = control('qr-code', 'settings.toggle_qr', () => {
      invoke('__hostToggleQrCode');
      refresh();
    });
    controls.qr = qr;
    qrRow.appendChild(qr);
    qrSection.appendChild(qrRow);
    body.appendChild(qrSection);

    const sessionSection = section('settings.gameplay.session');
    const sessionRow = rowHost();
    sessionRow.appendChild(
      control('exit-to-lobby', 'settings.gameplay.exit_to_lobby', () => {
        invoke('__hostReturnToLobby');
        close();
      }),
    );
    sessionSection.appendChild(sessionRow);
    body.appendChild(sessionSection);
  }

  // ── Panel ──────────────────────────────────────────────────────────────────

  function buildPanel() {
    const demo = isDemo();
    const tabs = visibleTabs(demo);
    // Shared with the phone client rather than duplicated: a panel whose active
    // tab was gated away renders an empty body instead of falling back, and the
    // two pages getting different answers to that is invisible until someone
    // opens the demo build's cog.
    activeTab = resolveActiveTab(activeTab, demo);
    controls.toggles = {};
    controls.commands = {};
    controls.outputs = {};
    controls.pause = null;
    controls.qr = null;
    controls.volumeReadout = null;

    overlay.innerHTML = '';

    const popup = doc.createElement('div');
    popup.className = 'server-settings-popup';
    overlay.appendChild(popup);

    const heading = doc.createElement('div');
    heading.className = 'server-settings-title';
    heading.textContent = t('settings.title');
    popup.appendChild(heading);

    const tabBar = doc.createElement('div');
    tabBar.className = 'server-settings-tabs';
    popup.appendChild(tabBar);

    const body = doc.createElement('div');
    body.className = 'server-settings-body';
    popup.appendChild(body);

    for (const tab of tabs) {
      const el = doc.createElement('button');
      el.type = 'button';
      el.className = 'server-settings-tab' + (tab.id === activeTab ? ' active' : '');
      el.setAttribute('data-tab', tab.id);
      el.textContent = t(tab.labelId);
      el.addEventListener('click', (e) => {
        if (e && typeof e.preventDefault === 'function') e.preventDefault();
        selectTab(tab.id);
      });
      tabBar.appendChild(el);
    }

    if (activeTab === 'debug') buildDebugTab(body);
    else if (activeTab === 'audio') buildAudioTab(body);
    else if (activeTab === 'gameplay') buildGameplayTab(body);

    paintOutput();
    refresh();
  }

  function selectTab(id) {
    activeTab = id;
    if (isOpen()) buildPanel();
  }

  /** Repaint everything whose truth lives in the simulation. */
  function refresh() {
    for (const entry of DEBUG_TOGGLES) {
      const el = controls.toggles[entry.id];
      if (!el) continue;
      const on = !!invoke(entry.state);
      el.classList.toggle('active', on);
      el.setAttribute('aria-pressed', on ? 'true' : 'false');
    }
    for (const entry of DEBUG_COMMANDS) {
      const el = controls.commands[entry.id];
      if (!el || !entry.enabled) continue;
      const ready = !!invoke(entry.enabled);
      el.disabled = !ready;
      el.classList.toggle('disabled', !ready);
    }
    if (controls.pause) {
      const paused = !!invoke('wasm_is_paused');
      controls.pause.textContent = t(
        paused ? 'settings.gameplay.resume' : 'settings.gameplay.pause',
      );
      controls.pause.classList.toggle('active', paused);
      controls.pause.setAttribute('aria-pressed', paused ? 'true' : 'false');
    }
    if (controls.qr) {
      const visible = !!invoke('__hostIsQrVisible');
      controls.qr.classList.toggle('active', visible);
      controls.qr.setAttribute('aria-pressed', visible ? 'true' : 'false');
    }
    // The output panel keeps streaming while it is open, panel or no panel.
    if (outputs.viewing) paintOutput();
  }

  function isOpen() {
    return overlay.hidden === false;
  }

  function open() {
    // Rebuilt on every open so the demo gate is re-evaluated: the WASM getter
    // that answers it is not published until the bundle boots (the stamped
    // meta tag covers that window, but the rebuild costs nothing).
    buildPanel();
    overlay.hidden = false;
    overlay.setAttribute('aria-hidden', 'false');
    overlay.classList.add('open');
    btn.setAttribute('aria-expanded', 'true');
  }

  function close() {
    overlay.hidden = true;
    overlay.setAttribute('aria-hidden', 'true');
    overlay.classList.remove('open');
    btn.setAttribute('aria-expanded', 'false');
  }

  btn.addEventListener('click', (e) => {
    if (e && typeof e.preventDefault === 'function') e.preventDefault();
    if (isOpen()) close();
    else open();
  });
  overlay.addEventListener('click', (e) => {
    if (e && e.target === overlay) close();
  });

  paintOutput();

  if (autoRefresh && win && typeof win.requestAnimationFrame === 'function') {
    const loop = () => {
      if (isOpen() || outputs.viewing) refresh();
      rafHandle = win.requestAnimationFrame(loop);
    };
    rafHandle = win.requestAnimationFrame(loop);
  }

  function destroy() {
    if (rafHandle !== null && win && typeof win.cancelAnimationFrame === 'function') {
      win.cancelAnimationFrame(rafHandle);
    }
    rafHandle = null;
    close();
  }

  return { open, close, isOpen, refresh, selectTab, destroy };
}

// Expose for the classic-script bootstrap in server.html.
if (typeof window !== 'undefined') {
  window.mountServerSettings = mountServerSettings;
}
