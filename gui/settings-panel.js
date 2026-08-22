/**
 * gui/settings-panel.js — the phone client's settings cog (issue #940).
 *
 * The mirror of the host page's cog (issue #939, `gui/server-settings.js`):
 * a gear top-left opening the same three tabs, gating the same one in a demo
 * build. The tab list itself is shared — `gui/settings-tabs.js` — so the two
 * pages cannot disagree about which controls exist.
 *
 *   - **Debug / Cheat** — the host's debug overlays plus God Mode, driven from
 *     the phone. Absent entirely in the public demo build, and so is the
 *     server-side route behind it: `ClientMessage::ToggleDebugFlag` carries
 *     `#[cfg(not(phoenix_demo_build))]`, so a demo binary cannot even decode
 *     the frame, and `command_admission::debug_route` is `#[cfg]`-split for
 *     God Mode's admission branch.
 *   - **Audio** — master volume, which SCALES each audio element's authored
 *     level rather than replacing it, exactly as the host page does.
 *   - **Gameplay** — the station rating, the viewscreen QR toggle, leaving your
 *     station, and — **in a dev build only** — pause/resume.
 *
 * That last exception is deliberate and it is not a tab-level gate. The tab
 * itself ships in every build because the demo needs the rest of it; the pause
 * control alone is hidden when `isDemoBuild()` is true, matching
 * `ClientMessage::TogglePause`, which is compiled out of a demo binary. A demo
 * is N strangers on N phones, and nothing on the server side checks station,
 * captaincy or game phase before honouring a pause — so any one of them could
 * otherwise freeze the mission for everyone, over and over. The HOST's pause,
 * on its own cog (issue #939), is untouched in every build: that is one trusted
 * operator standing at the viewscreen.
 *
 * Exit-to-lobby is deliberately absent: it is host-side authority. A phone can
 * already *request* it (`ReturnToLobby` from the game-over overlay) and
 * `handler::may_return_to_lobby` allows a participant only during `GameOver`,
 * which is the correct existing behaviour — #940 does not loosen it.
 *
 * Everything the panel shows about the simulation comes from `getState()`,
 * which client.html composes from the lobby mirror and `window.simState`. The
 * debug flags in particular are the SERVER's read-back
 * (`ServerMessage::DebugState`, folded by `gui/sim-state.js`), never local
 * optimism — a demo build refuses the toggle outright and the button has to
 * show that rather than lighting up regardless.
 *
 * Pure JS, no WASM: the phone has none. Every state decision below is a pure
 * exported function so vitest drives it without a DOM.
 */

import { t, wireText } from './strings.js';
import { isDemoBuild } from './build-flags.js';
import { visibleClientTabs, resolveClientActiveTab } from './settings-tabs.js';
import { controlSystemEnvelope } from './command-gateway.js';
import { renderStationHelp } from './help-panel.js';
import { renderManual } from './manual-panel.js';
import { TEXT_SCALE_MIN, TEXT_SCALE_MAX, TEXT_SCALE_STEP } from './accessibility-profile.js';
import {
  mountOverlayShell,
  renderTabBar,
  makeSectionBuilders,
  makeRowBuilder,
  VOLUME_MIN,
  VOLUME_MAX,
  VOLUME_STEP,
} from './settings-overlay-kit.js';

/** localStorage key for the master volume. Unchanged from the pre-#940 slider. */
const STORAGE_KEY = 'phoenix-settings-volume';

// Master volume's 0..1 range and percent resolution are shared with the host
// cog — see `gui/settings-overlay-kit.js` — 1.0 is the identity (every
// channel at its authored level), which is why it is also the default.

/**
 * The Debug/Cheat tab's overlay toggles.
 *
 * `flag` is the `DebugFlag` variant name sent in
 * `ClientMessage::ToggleDebugFlag` — the spelling is the Rust enum's, pinned by
 * `codec::client_settings_menu_wire_shapes_are_pinned`. Labels are reused from
 * the host cog so both pages name the same control the same way.
 *
 * Every entry is diagnostic-only, and that is now true by construction rather
 * than by convention: pause used to be a `DebugFlag` and is a message of its
 * own since #940, which is what lets the whole `ToggleDebugFlag` route be
 * compiled out of a demo build instead of narrowed flag by flag.
 */
export const CLIENT_DEBUG_FLAGS = [
  { id: 'wireframes', labelId: 'settings.debug.wireframes', flag: 'Regions' },
  { id: 'modifiers', labelId: 'settings.debug.modifiers', flag: 'Modifiers' },
  { id: 'damage', labelId: 'settings.debug.damage', flag: 'Damage' },
  { id: 'entities', labelId: 'settings.debug.entities', flag: 'Entities' },
  { id: 'inspector', labelId: 'settings.debug.inspector', flag: 'Inspector' },
  // Station activity (issue #1145) renders only on the host viewscreen, but the
  // phone still toggles the flag — the client keeps its toggle-only role even
  // where it cannot draw the surface (PRD #1144).
  { id: 'station-activity', labelId: 'settings.debug.station_activity', flag: 'StationActivity' },
  // AI doctrine pool (issue #1149) renders only on the host viewscreen, but the
  // phone still toggles the flag — the client keeps its toggle-only role.
  { id: 'ai-doctrine', labelId: 'settings.debug.ai_doctrine', flag: 'AiDoctrine' },
];

/**
 * The Gameplay tab's pause control, as a `data-control` id.
 *
 * There is no `PAUSE_FLAG` any more: pause is not a `DebugFlag`, it is
 * `ClientMessage::TogglePause`, a message of its own so that it can be compiled
 * out of a demo build independently of the debug overlays.
 */
export const PAUSE_CONTROL_ID = 'pause';

/**
 * The `SystemId` God Mode is addressed by — `system_registry::GOD_MODE_SYSTEM_ID`.
 * Ownerless: no ship TOML declares it, which is exactly why it needed the
 * `debug_route` branch in `command_admission::policy` to be reachable at all.
 */
export const GOD_MODE_SYSTEM_ID = 'god-mode';

// ── Message builders ────────────────────────────────────────────────────────

/**
 * The `ClientMessage::ToggleDebugFlag` envelope for one flag.
 *
 * A top-level client message rather than a `ControlSystem` payload: these are
 * session controls, not ship-system commands. See the Rust variant's doc.
 *
 * @param {string} flag — a `DebugFlag` variant name, e.g. `'Regions'`.
 * @returns {{type: string, data: {flag: string}}}
 */
export function debugFlagMessage(flag) {
  if (typeof flag !== 'string' || flag.length === 0) {
    throw new TypeError('settings-panel: debug flag must be a non-empty string');
  }
  return { type: 'ToggleDebugFlag', data: { flag } };
}

/**
 * The `ClientMessage::TogglePause` envelope.
 *
 * A message of its own rather than a `DebugFlag`, because it needs its own
 * build gate: the Rust variant carries `#[cfg(not(phoenix_demo_build))]`, so a
 * demo binary does not understand this frame at all. Sending it from a demo
 * client would not pause anything — which is why the control that sends it is
 * not rendered there either. See the module doc.
 *
 * A unit variant on the wire, so it carries NO `data` key — the frame is
 * `{"type":"TogglePause"}`, which is what `connection-manager.js` emits when
 * `send()` is given no data, and the same shape `ReleaseStation` has always
 * used. `data: {}` would be a different message and the host would reject it;
 * `codec::client_settings_menu_wire_shapes_are_pinned` pins both facts.
 *
 * @returns {{type: string}}
 */
export function pauseMessage() {
  return { type: 'TogglePause' };
}

/**
 * The `ControlSystem` envelope for God Mode.
 *
 * Unlike the overlays this one really does cross command admission (issue
 * #900): God Mode changes damage outcomes, so it has to be tick-stamped,
 * logged and replayable. Built through `gui/command-gateway.js` so the phone
 * has exactly one place that knows the `ControlSystem` shape.
 *
 * @returns {{type: string, data: {target: string, payload: object}}}
 */
export function godModeMessage() {
  return controlSystemEnvelope(GOD_MODE_SYSTEM_ID, { type: 'ToggleGodMode' });
}

// ── State builder ───────────────────────────────────────────────────────────

/**
 * Fold the client's state into everything the panel renders.
 *
 * Pure, so the awkward parts — which tab survives the build, which rating is
 * active, what a debug button shows before the server has ever reported —
 * are tested without a DOM.
 *
 * `debug` is the server's read-back as `gui/sim-state.js` folds it:
 * `{ flags: {Regions: bool, ...}, godMode: bool }`, or `null` before the first
 * `DebugState` arrives. A flag the server has not reported renders OFF and
 * un-pressed; it never guesses.
 *
 * @param {{
 *   state?: object,      // composed client state (stations, stationRatings, debugFlags)
 *   myToken?: string|null,
 *   demo?: boolean,
 *   activeTab?: string|null,
 * }} opts
 * @returns {{
 *   tabs: Array<{id: string, labelId: string}>,
 *   activeTab: string|null,
 *   stationId: string|null,
 *   ratings: Array<{name: string, label: string, active: boolean}>,
 *   debugFlags: Array<{id: string, labelId: string, flag: string, on: boolean}>,
 *   godMode: boolean,
 *   paused: boolean,
 *   showPause: boolean,
 *   reported: boolean,
 * }}
 */
export function buildSettingsState(opts = {}) {
  const state = opts.state || {};
  const demo = !!opts.demo;
  const myToken = opts.myToken || null;

  const stations = state.stations || [];
  const myStation = stations.find((st) => st.holder_token === myToken) || null;
  const stationId = myStation ? myStation.id : null;
  // AFK presence (issue #1104) is a PUBLIC per-player flag on the roster, so the
  // tab paints the toggle from server truth (the local player's own record),
  // never from what was last clicked. A silent/legacy roster defaults to false.
  const players = state.players || [];
  const mePlayer = players.find((p) => p.token === myToken) || null;
  const afk = !!(mePlayer && mePlayer.afk);
  const names = (myStation && myStation.ratings) || [];
  const stationRatings = state.stationRatings || {};
  const activeRating = (stationId && stationRatings[stationId]) || names[0] || '';

  // Rating names are lookup identifiers in the ship TOML (Rust matches them by
  // name), so display text comes from a derived string id and falls back to the
  // identifier upper-cased when no string is authored for it.
  const ratings = names.map((name) => {
    const key = 'station.rating.' + String(name).toLowerCase() + '.name';
    return {
      name,
      label: wireText(key, String(name).toUpperCase()),
      active: name === activeRating,
    };
  });

  const debug = state.debugFlags || null;
  const reported = !!debug;
  const flags = (debug && debug.flags) || {};

  return {
    tabs: visibleClientTabs(demo).map((tab) => ({ id: tab.id, labelId: tab.labelId })),
    activeTab: resolveClientActiveTab(opts.activeTab || null, demo),
    stationId,
    afk,
    ratings,
    // The private Accessibility profile (issue #1102), reflected as the
    // player's EXPLICIT choices so the tab paints which option is selected.
    // Pure: this reads the stored tri-states only — the OS default and the
    // resolved effect are an apply()-time concern (gui/accessibility-profile.js),
    // never computed here.
    accessibility: accessibilityView(state.accessibilityProfile),
    debugFlags: CLIENT_DEBUG_FLAGS.map((entry) => ({
      ...entry,
      on: !!flags[entry.flag],
    })),
    godMode: !!(debug && debug.godMode),
    paused: !!(debug && debug.paused),
    // The pause control, not the Gameplay tab, is what the demo build hides —
    // the tab carries the rating, QR and leave-station controls a demo needs.
    // Mirrors `ClientMessage::TogglePause`'s `#[cfg]`: a demo binary cannot
    // decode the frame, so offering the button would be offering a dead one.
    showPause: !demo,
    reported,
  };
}

/** The three tri-states an OS-defaultable accessibility effect can carry. */
const A11Y_TRI_STATES = new Set(['default', 'on', 'off']);

/**
 * Fold the stored accessibility profile into what the tab paints. Pure: it
 * reflects the player's explicit choices only. `textScaleValue` is the slider's
 * position — the numeric scale, or the identity (1) when the effect is unset.
 *
 * @param {object|null} profile
 */
export function accessibilityView(profile) {
  const pres = (profile && profile.presentation) || {};
  const textScale = pres.textScale;
  const tri = (v) => (A11Y_TRI_STATES.has(v) ? v : 'default');
  return {
    textScale: typeof textScale === 'number' ? textScale : 'default',
    textScaleValue: typeof textScale === 'number' ? textScale : 1,
    contrast: tri(pres.contrast),
    reducedMotion: tri(pres.reducedMotion),
  };
}

// ── Master volume ───────────────────────────────────────────────────────────

/**
 * A master volume that SCALES each channel's authored level.
 *
 * The same shape the host page uses (issue #939 keeps each channel's TOML level
 * in `_authoredVol` and always emits `authored × master`), for the same reason:
 * replacing the level instead of scaling it would flatten every channel's
 * authored balance, and a master of 1.0 would stop being a no-op.
 *
 * The client's authored level is whatever the audio element already carries
 * when it is registered — the media element's own `volume`, which the page or a
 * future config sets. This module never invents one, so there is no gameplay
 * value here to hardcode.
 *
 * @param {Array<{volume: number}>} elements — audio elements (or any object
 *   with a numeric `volume`, which is what the tests pass).
 * @param {number} initial — starting master, 0..1.
 * @returns {{ get: function, set: function, channels: Array }}
 */
export function createMasterVolume(elements, initial) {
  const channels = (elements || [])
    .filter(Boolean)
    .map((el) => ({ el, authored: typeof el.volume === 'number' ? el.volume : VOLUME_MAX }));
  let master = clampVolume(initial);

  const apply = () => {
    for (const channel of channels) channel.el.volume = channel.authored * master;
  };
  apply();

  return {
    get: () => master,
    set: (value) => {
      master = clampVolume(value);
      apply();
      return master;
    },
    channels,
  };
}

/** Clamp to the slider's range, treating a non-number as "no scaling". */
export function clampVolume(value) {
  const v = Number(value);
  if (!Number.isFinite(v)) return VOLUME_MAX;
  return Math.min(VOLUME_MAX, Math.max(VOLUME_MIN, v));
}

/** Read the persisted master volume, or the identity when there is none. */
function storedMasterVolume() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw === null ? VOLUME_MAX : clampVolume(parseFloat(raw));
  } catch (_) {
    return VOLUME_MAX;
  }
}

/** Persist the master volume, ignoring a storage that refuses writes. */
function persistMasterVolume(value) {
  try {
    localStorage.setItem(STORAGE_KEY, String(value));
  } catch (_) {
    /* private-mode Safari and friends — the session still works, it just
       forgets the setting. */
  }
}

// ── Mount ───────────────────────────────────────────────────────────────────

/**
 * Mount the cog and its panel on `doc`.
 *
 * @param {{
 *   send?: function,             // (type, data) onto the wire
 *   getState?: function,         // composed client state
 *   audioEl?: object|null,       // legacy single-channel argument
 *   audioEls?: Array,            // every audio channel master volume scales
 *   myToken?: string|null,
 *   doc?: Document,
 *   isDemo?: () => boolean,
 * }} opts
 * @returns {{ open: function, close: function, rebuildContent: function,
 *             selectTab: function, isOpen: function }}
 */
export function mountSettings({
  send,
  getState,
  audioEl,
  audioEls,
  getManual,
  myToken,
  onAccessibility: _onAccessibility,
  doc: _doc,
  isDemo: _isDemo,
} = {}) {
  const doc = _doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) {
    return {
      open() {}, close() {}, rebuildContent() {},
      selectTab() {}, isOpen: () => false,
    };
  }
  const win = doc.defaultView || (typeof window !== 'undefined' ? window : null);
  const isDemo = _isDemo || (() => isDemoBuild({ win, doc }));

  let activeTab = null;

  // The documentation surface's own state, deliberately NOT `activeTab`.
  //
  // The panel keeps one tab slot, and the Ship Manual has a second tab strip
  // INSIDE it — one tab per station. That inner selection used to live only in
  // a closure created by `renderManual`, which meant it existed for exactly as
  // long as the DOM `buildContent()` had most recently thrown away. Every
  // settings-driven repaint therefore reset the reader to the first station:
  // switching Settings tabs and back, and — worse, because it is not the
  // reader's doing — any `DebugState` push, which client.html forwards to
  // `rebuildContent()` while the panel is open.
  //
  // A debug flag changing on the host has nothing to do with which page of the
  // manual a player is reading. Settings state and documentation state are
  // separate facts, so they get separate slots: `renderManual` is handed the
  // remembered index and reports back when the reader moves.
  let manualStationIndex = 0;

  // Master volume owns every audio channel the page hands it. `audioEl` is the
  // pre-#940 single-element argument, kept working so client.html's existing
  // call site did not have to change in the same commit as the panel.
  const elements = (audioEls && audioEls.length ? audioEls : [audioEl]).filter(Boolean);
  const master = createMasterVolume(elements, storedMasterVolume());

  const emit = (type, data) => {
    if (typeof send === 'function') send(type, data);
  };

  // The Accessibility tab's write path (issue #1102). DELIBERATELY separate
  // from `emit`/`send`: a presentation choice is client-local and must never
  // become a ClientMessage (AC5). client.html wires this to
  // window.setAccessibilityPresentation (update simState, persist privately,
  // re-apply to the shell + console iframes); absent, it is a harmless no-op.
  const setAccessibility = (effect, value) => {
    if (typeof _onAccessibility === 'function') _onAccessibility(effect, value);
  };

  // ── Gear button + overlay ────────────────────────────────────────────────
  //
  // The find-or-create/aria/focus-trap/open-close/backdrop-click mechanics are
  // shared with the host cog (issue #1238) — see `gui/settings-overlay-kit.js`
  // for what stayed here vs there.
  const shell = mountOverlayShell(doc, {
    buttonId: 'settings-btn',
    overlayId: 'settings-overlay',
    buttonClass: 'settings-btn',
    overlayClass: 'settings-overlay',
    // The gear sits over consoles that have their own click handling; the
    // host page has no such layer beneath its cog, so this stays client-only.
    stopPropagationOnToggle: true,
  });
  const { overlay } = shell;
  // `buildContent` is a hoisted function declaration, so this may run before
  // its textual definition below.
  shell.buildContent = buildContent;

  // ── Small builders ───────────────────────────────────────────────────────

  const { section, hint } = makeSectionBuilders(doc, {
    sectionClass: 'settings-section',
    headingClass: 'settings-section-heading',
    hintClass: 'settings-section-hint',
  });
  const row = makeRowBuilder(doc, 'settings-row');

  /** A toggle button whose pressed state is painted from server truth. */
  function toggle(id, label, on, onClick) {
    const el = doc.createElement('button');
    el.className = 'settings-rating-btn' + (on ? ' active' : '');
    el.setAttribute('data-control', id);
    el.setAttribute('aria-pressed', on ? 'true' : 'false');
    el.textContent = label;
    el.addEventListener('click', (e) => {
      if (e && typeof e.preventDefault === 'function') e.preventDefault();
      onClick();
    });
    return el;
  }

  function action(label, extraClass, onClick) {
    const el = doc.createElement('button');
    el.className = 'settings-action-btn' + (extraClass ? ' ' + extraClass : '');
    el.textContent = label;
    el.addEventListener('click', (e) => {
      if (e && typeof e.preventDefault === 'function') e.preventDefault();
      onClick();
    });
    return el;
  }

  // ── Tab bodies ───────────────────────────────────────────────────────────

  function buildDebugTab(body, view) {
    const overlays = section('settings.debug.output');
    // The phone has nowhere to show a debug stream — these flags draw on the
    // shared host viewscreen. Saying so is the difference between "nothing
    // happened" and "look up".
    overlays.appendChild(hint('settings.debug.client_hint'));
    const overlayRow = row('settings-rating-row');
    for (const entry of view.debugFlags) {
      overlayRow.appendChild(
        toggle(entry.id, t(entry.labelId), entry.on, () => {
          // Fire and repaint from the next `DebugState`, never optimistically:
          // in a demo build there is no route and the server will report the
          // flag unchanged, which is the honest thing for the button to show.
          emit('ToggleDebugFlag', debugFlagMessage(entry.flag).data);
        }),
      );
    }
    overlays.appendChild(overlayRow);
    body.appendChild(overlays);

    const cheats = section('settings.debug.cheats');
    const cheatRow = row('settings-rating-row');
    cheatRow.appendChild(
      toggle('godmode', t('settings.debug.godmode'), view.godMode, () => {
        const envelope = godModeMessage();
        emit(envelope.type, envelope.data);
      }),
    );
    cheats.appendChild(cheatRow);
    body.appendChild(cheats);
  }

  function buildAudioTab(body) {
    const el = section('settings.master_volume');
    const volRow = row('settings-vol-row');

    const slider = doc.createElement('input');
    slider.type = 'range';
    slider.min = String(VOLUME_MIN);
    slider.max = String(VOLUME_MAX);
    slider.step = String(VOLUME_STEP);
    slider.value = String(master.get());

    const label = doc.createElement('span');
    label.className = 'settings-vol-label';

    const paint = () => {
      label.textContent = t('settings.master_volume_value', {
        value: String(Math.round(master.get() * 100)),
      });
    };
    // `input`, not `change`: the acceptance criterion is that the level moves
    // as you drag, not when you let go.
    slider.addEventListener('input', function () {
      const applied = master.set(this.value);
      persistMasterVolume(applied);
      paint();
    });
    paint();

    volRow.appendChild(slider);
    volRow.appendChild(label);
    el.appendChild(volRow);
    el.appendChild(hint('settings.master_volume_hint'));
    body.appendChild(el);
  }

  // A row of tri-state option buttons for one OS-defaultable effect. Exactly
  // one is active (the player's explicit choice); each writes its value on the
  // client-local path — never a ClientMessage.
  function accessibilityChoiceRow(effect, current, options) {
    const rowEl = row('settings-rating-row');
    for (const [value, labelId] of options) {
      rowEl.appendChild(
        toggle('a11y-' + effect + '-' + value, t(labelId), current === value, () => {
          setAccessibility(effect, value);
          // Repaint so the active option moves. Safe here (unlike the sliders):
          // no drag is in flight on a button press.
          buildContent();
        }),
      );
    }
    return rowEl;
  }

  function buildAccessibilityTab(body, view) {
    const a = view.accessibility;

    // Explanatory copy: names effects, states the profile is private/local, and
    // never asks for or infers a diagnosis or a reason (AC1).
    const intro = section('settings.accessibility.presentation');
    intro.appendChild(hint('settings.accessibility.intro_hint'));
    intro.appendChild(hint('settings.accessibility.local_hint'));
    body.appendChild(intro);

    // Text size — the observable effect proven end to end (AC3). Drives
    // --a11y-text-scale on every console :root via the client-local path.
    const textSec = section('settings.accessibility.text_scale');
    const scaleRow = row('settings-vol-row');

    const slider = doc.createElement('input');
    slider.type = 'range';
    slider.min = String(TEXT_SCALE_MIN);
    slider.max = String(TEXT_SCALE_MAX);
    slider.step = String(TEXT_SCALE_STEP);
    slider.value = String(a.textScaleValue);
    slider.setAttribute('data-control', 'a11y-text-scale');

    const label = doc.createElement('span');
    label.className = 'settings-vol-label';
    const paint = () => {
      label.textContent = t('settings.accessibility.text_scale_value', {
        value: String(Math.round(Number(slider.value) * 100)),
      });
    };
    // `input`, not `change`: the console text must resize under the finger, and
    // we do NOT rebuild the panel (that would drop the drag) — the readout is
    // updated locally, exactly as the master-volume slider does.
    slider.addEventListener('input', function () {
      setAccessibility('textScale', Number(this.value));
      paint();
    });
    paint();

    scaleRow.appendChild(slider);
    scaleRow.appendChild(label);
    textSec.appendChild(scaleRow);
    textSec.appendChild(hint('settings.accessibility.text_scale_hint'));
    textSec.appendChild(
      action(t('settings.accessibility.text_scale_reset'), null, () => {
        setAccessibility('textScale', 'default');
        buildContent();
      }),
    );
    body.appendChild(textSec);

    // Contrast — tri-state: follow the OS, force more, or force standard.
    const contrastSec = section('settings.accessibility.contrast');
    contrastSec.appendChild(accessibilityChoiceRow('contrast', a.contrast, [
      ['default', 'settings.accessibility.follow_system'],
      ['on', 'settings.accessibility.contrast_more'],
      ['off', 'settings.accessibility.contrast_standard'],
    ]));
    contrastSec.appendChild(hint('settings.accessibility.contrast_hint'));
    body.appendChild(contrastSec);

    // Motion — tri-state: follow the OS, reduce, or allow full motion even when
    // the OS asks to reduce (the explicit override wins both ways).
    const motionSec = section('settings.accessibility.reduced_motion');
    motionSec.appendChild(accessibilityChoiceRow('reducedMotion', a.reducedMotion, [
      ['default', 'settings.accessibility.follow_system'],
      ['on', 'settings.accessibility.motion_reduce'],
      ['off', 'settings.accessibility.motion_allow'],
    ]));
    motionSec.appendChild(hint('settings.accessibility.reduced_motion_hint'));
    body.appendChild(motionSec);
  }

  function buildGameplayTab(body, view) {
    // Dev builds only — see the module doc. The whole section goes, not just
    // the button: a "Simulation" heading over nothing reads as a bug.
    if (view.showPause) {
      const sim = section('settings.gameplay.simulation');
      const simRow = row('settings-rating-row');
      simRow.appendChild(
        toggle(
          PAUSE_CONTROL_ID,
          t(view.paused ? 'settings.gameplay.resume' : 'settings.gameplay.pause'),
          view.paused,
          () => {
            const envelope = pauseMessage();
            emit(envelope.type, envelope.data);
          },
        ),
      );
      sim.appendChild(simRow);
      body.appendChild(sim);
    }

    // Rating, QR and Leave Station predate the tabs (they were the whole of the
    // pre-#940 panel). They are session controls, so they belong on the tab
    // that is never build-gated.
    if (view.stationId && view.ratings.length > 1) {
      const ratingSection = section('settings.rating');
      const ratingRow = row('settings-rating-row');
      for (const rating of view.ratings) {
        ratingRow.appendChild(
          toggle('rating-' + rating.name, rating.label, rating.active, () => {
            if (!rating.active) emit('SetStationRating', { rating_name: rating.name });
          }),
        );
      }
      ratingSection.appendChild(ratingRow);
      body.appendChild(ratingSection);
    }

    const qrSection = section('settings.qr_code');
    qrSection.appendChild(
      action(t('settings.toggle_qr'), null, () => emit('ToggleQrCode', {})),
    );
    body.appendChild(qrSection);

    // AFK (issue #1104): step away from a HELD station, delegating its systems
    // to AI without giving up the seat. Only meaningful while holding a station,
    // so it rides alongside Leave Station (which is likewise station-gated). The
    // toggle is painted from server truth (`view.afk`) and emits the flipped
    // flag; unlike Leave it does NOT close the panel, so a player can return the
    // same way they left.
    if (view.stationId) {
      const afkSection = section('settings.afk');
      const afkRow = row('settings-rating-row');
      afkRow.appendChild(
        toggle(
          'afk-toggle',
          t(view.afk ? 'settings.afk_active' : 'settings.afk_enter'),
          view.afk,
          () => emit('SetAfk', { afk: !view.afk }),
        ),
      );
      afkSection.appendChild(afkRow);
      body.appendChild(afkSection);
    }

    if (view.stationId) {
      const leaveSection = section('settings.station');
      leaveSection.appendChild(
        action(t('settings.leave_station'), 'settings-leave-btn', () => {
          shell.close();
          emit('ReleaseStation');
        }),
      );
      body.appendChild(leaveSection);
    }
  }

  function buildStationHelpTab(body, view) {
    const host = doc.createElement('div');
    host.className = 'settings-documentation';
    if (!view.stationId || !renderStationHelp(host, view.stationId)) {
      const unavailable = doc.createElement('div');
      unavailable.className = 'settings-section-hint';
      unavailable.textContent = t('settings.station_help.unavailable');
      host.appendChild(unavailable);
    }
    body.appendChild(host);
  }

  function buildShipManualTab(body) {
    const host = doc.createElement('div');
    host.className = 'settings-documentation';
    const manual = typeof getManual === 'function' ? getManual() : null;
    const stations = renderManual(host, manual, manualStationIndex, (index) => {
      manualStationIndex = index;
    });
    // A shorter manual (a different ship) must not leave the remembered index
    // pointing past the end of the new one.
    if (stations > 0 && manualStationIndex >= stations) manualStationIndex = stations - 1;
    body.appendChild(host);
  }

  // ── Panel ────────────────────────────────────────────────────────────────

  function buildContent() {
    const view = buildSettingsState({
      state: getState ? getState() : {},
      myToken,
      demo: !!isDemo(),
      activeTab,
    });
    activeTab = view.activeTab;

    overlay.innerHTML = '';

    const popup = doc.createElement('div');
    popup.className = 'settings-popup';
    overlay.appendChild(popup);

    const tabBar = doc.createElement('div');
    tabBar.className = 'settings-tabs';
    popup.appendChild(tabBar);

    const body = doc.createElement('div');
    body.className = 'settings-body';
    popup.appendChild(body);

    renderTabBar(doc, tabBar, view.tabs, activeTab, 'settings-tab', selectTab);

    if (activeTab === 'debug') buildDebugTab(body, view);
    else if (activeTab === 'audio') buildAudioTab(body);
    else if (activeTab === 'accessibility') buildAccessibilityTab(body, view);
    else if (activeTab === 'gameplay') buildGameplayTab(body, view);
    else if (activeTab === 'station-help') buildStationHelpTab(body, view);
    else if (activeTab === 'ship-manual') buildShipManualTab(body);
  }

  function selectTab(id) {
    activeTab = id;
    if (shell.isOpen()) buildContent();
  }

  // `rebuildContent` repaints an OPEN panel from current state. client.html
  // calls it when a `DebugState` arrives, which is the only push that can
  // change what the panel shows while it is open. Deliberately event-driven
  // rather than per-frame: a rebuild replaces the volume slider element, and
  // doing that under a dragging finger would drop the drag.
  return {
    open: shell.open,
    close: shell.close,
    isOpen: shell.isOpen,
    selectTab,
    rebuildContent: () => {
      if (shell.isOpen()) buildContent();
    },
  };
}

// Expose for non-module scripts (fallback).
if (typeof window !== 'undefined') {
  window.mountSettings = mountSettings;
}
