/**
 * gui/viewscreen-presentation-panel.js — the Viewscreen settings menu's
 * Display tab body, built once for both viewscreen runtimes (issue #1427).
 *
 * `gui/server-settings.js` (browser `server.html`) and `gui/native-settings.js`
 * (the native host's lobby/viewscreen window) are two cogs over ONE display
 * concept, and the controls on this tab are the same controls: the same slider
 * bounds, the same tri-state, the same live preview, the same two reset scopes,
 * the same status vocabulary. Writing them twice would be two chances to let a
 * reset stop being scoped, or a preview stop being live, on the surface nobody
 * happened to open.
 *
 * What the two surfaces genuinely differ in is their CSS vocabulary and their
 * element factories — `.server-settings-*` against `.native-settings-*` — so
 * those arrive as arguments, exactly the way `createSemanticControlsRemapper`
 * takes `section`/`hint`/`row`/`action` from whichever panel is mounting it.
 * This module owns the CONTROLS and their behaviour; the caller owns how they
 * are dressed.
 *
 * ## It repaints itself rather than asking for a rebuild
 *
 * Every control here writes through the controller and then repaints only the
 * nodes whose text changed. That is not an optimisation:
 *
 *  * a slider drag must survive its own `input` events — rebuilding the panel
 *    under the finger ends the drag, which is the live-preview requirement
 *    (PRD #1418 story 16) failing in the one interaction that needs it most;
 *  * a tri-state press must leave focus on the button that was pressed, so a
 *    keyboard operator can press System, hear the status line, and press again.
 *    A rebuilt overlay drops focus to the body (issue #1422 found and fixed the
 *    same thing on the phone's Accessibility tab).
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

import {
  TEXT_SCALE_MIN,
  TEXT_SCALE_MAX,
  TEXT_SCALE_STEP,
  FOLLOW_OS,
  EXPLICIT_ON,
  EXPLICIT_OFF,
} from './accessibility-profile.js';

/**
 * `data-control` ids, exported so a test, a smoke spec and the CSS all name the
 * same controls rather than three copies of the same string.
 */
export const VIEWSCREEN_PRESENTATION_CONTROLS = Object.freeze({
  textScale: 'viewscreen-text-scale',
  textScaleStatus: 'viewscreen-text-scale-status',
  textScaleReset: 'viewscreen-text-scale-reset',
  contrast: (value) => `viewscreen-contrast-${value}`,
  contrastStatus: 'viewscreen-contrast-status',
  contrastReset: 'viewscreen-contrast-reset',
  resetAll: 'viewscreen-reset-all',
});

/** Which of the three sources a live value came from, in words. The vocabulary
 *  is `presentationStatus`'s; the copy is this endpoint's own, because "your
 *  choice" is the wrong sentence for a screen a whole room shares. */
const SOURCE_LABELS = Object.freeze({
  explicit: 'settings.viewscreen.source_explicit',
  system: 'settings.viewscreen.source_system',
  default: 'settings.viewscreen.source_default',
});

/** The three contrast options, in display order. Follow-the-system first: it is
 *  the default, and a reset returns here. */
const CONTRAST_OPTIONS = Object.freeze([
  [FOLLOW_OS, 'settings.viewscreen.follow_system'],
  [EXPLICIT_ON, 'settings.viewscreen.contrast_more'],
  [EXPLICIT_OFF, 'settings.viewscreen.contrast_standard'],
]);

/** Whole percent, the resolution both viewscreen menus offer text size at. */
function percent(value) {
  return String(Math.round(Number(value) * 100));
}

/**
 * Build the Display tab body into `target`.
 *
 * @param {Element} target the tab body to append into.
 * @param {{
 *   doc: Document,
 *   t: (id: string, params?: object) => string,
 *   presentation: object,           // gui/viewscreen-presentation.js controller
 *   section: (labelId: string) => Element,
 *   hint: (labelId: string) => Element,
 *   row: (className?: string) => Element,
 *   control: (id: string, labelId: string, onClick: function) => Element,
 *   classes?: { slider?: string, readout?: string, status?: string },
 * }} opts
 * @returns {{ repaint: () => void }} `repaint` re-reads the controller and
 *   updates the pressed states, the readout and both status lines in place.
 *   Returned rather than only used internally so a surface whose record can
 *   change from somewhere else — a native host replying to a save, a second
 *   press of a key — can say so without rebuilding the panel.
 */
export function renderViewscreenPresentationPanel(target, opts) {
  const { doc, t, presentation, section, hint, row, control } = opts;
  const classes = opts.classes || {};
  const sliderClass = classes.slider || '';
  const readoutClass = classes.readout || '';
  const statusClass = classes.status || '';

  const nodes = { contrast: {} };

  // ── What this tab is, and whose it is ────────────────────────────────────
  //
  // Two sentences, and the second is the one that matters: an operator standing
  // at the shared screen has to be able to tell that this is not the private
  // profile they set on their phone, and that what they change here stays with
  // the room rather than following anybody home.
  const intro = section('settings.viewscreen.presentation');
  intro.appendChild(hint('settings.viewscreen.intro_hint'));
  intro.appendChild(hint('settings.viewscreen.endpoint_hint'));
  const unavailable = presentation.unavailable();
  if (unavailable.length) {
    // A native read that FAILED is not the same as a system that said nothing,
    // and only the first is worth a line of explanation (issue #1422).
    const line = hint('settings.viewscreen.os_unavailable');
    line.setAttribute('data-control', 'viewscreen-os-unavailable');
    intro.appendChild(line);
  }
  target.appendChild(intro);

  // ── Text size ────────────────────────────────────────────────────────────
  //
  // The bounds are the shared contract's (`gui/accessibility-profile.js`,
  // 100%-200% since issue #1422), read from the constants and never restated:
  // the native bridge reasons over the same range, and a viewscreen offering a
  // scale the rest of the fleet does not support would be a second ceiling.
  const textSection = section('settings.viewscreen.text_scale');
  const scaleRow = row();

  const slider = doc.createElement('input');
  slider.type = 'range';
  if (sliderClass) slider.className = sliderClass;
  slider.min = String(TEXT_SCALE_MIN);
  slider.max = String(TEXT_SCALE_MAX);
  slider.step = String(TEXT_SCALE_STEP);
  slider.setAttribute('data-control', VIEWSCREEN_PRESENTATION_CONTROLS.textScale);
  slider.setAttribute('aria-label', t('settings.viewscreen.text_scale'));
  nodes.slider = slider;

  const readout = doc.createElement('span');
  if (readoutClass) readout.className = readoutClass;
  nodes.readout = readout;

  // `input`, not `change`: the menu has to resize under the finger while the
  // slider moves, which is what "immediate live preview" means for a value the
  // operator is judging by eye.
  slider.addEventListener('input', function () {
    presentation.set('textScale', Number(this.value));
    paintTextScale();
  });

  scaleRow.appendChild(slider);
  scaleRow.appendChild(readout);
  textSection.appendChild(scaleRow);
  textSection.appendChild(hint('settings.viewscreen.text_scale_hint'));
  nodes.textStatus = statusLine(VIEWSCREEN_PRESENTATION_CONTROLS.textScaleStatus);
  textSection.appendChild(nodes.textStatus);
  textSection.appendChild(resetButton(
    VIEWSCREEN_PRESENTATION_CONTROLS.textScaleReset,
    'settings.viewscreen.text_scale_reset',
    'textScale',
  ));
  target.appendChild(textSection);

  // ── Contrast ─────────────────────────────────────────────────────────────
  //
  // Three ordinary pressed buttons rather than a checkbox, because the middle
  // state is real: "follow this machine" is not "off", and an operator has to
  // be able to force standard contrast on a display whose OS asked for more.
  const contrastSection = section('settings.viewscreen.contrast');
  const contrastRow = row();
  for (const [value, labelId] of CONTRAST_OPTIONS) {
    const id = VIEWSCREEN_PRESENTATION_CONTROLS.contrast(value);
    const el = control(id, labelId, () => {
      presentation.set('contrast', value);
      paintContrast();
    });
    nodes.contrast[value] = el;
    contrastRow.appendChild(el);
  }
  contrastSection.appendChild(contrastRow);
  contrastSection.appendChild(hint('settings.viewscreen.contrast_hint'));
  nodes.contrastStatus = statusLine(VIEWSCREEN_PRESENTATION_CONTROLS.contrastStatus);
  contrastSection.appendChild(nodes.contrastStatus);
  contrastSection.appendChild(resetButton(
    VIEWSCREEN_PRESENTATION_CONTROLS.contrastReset,
    'settings.viewscreen.contrast_reset',
    'contrast',
  ));
  target.appendChild(contrastSection);

  // ── Reset all, scoped ────────────────────────────────────────────────────
  //
  // Its own section with its own two hints, because a panel carrying more than
  // one "Reset all" is how an operator loses a set of key bindings while trying
  // to undo a text size. This one can only reach the two controls above it: the
  // record it clears contains nothing else (see viewscreen-presentation.js).
  const resetSection = section('settings.viewscreen.reset_all_heading');
  resetSection.appendChild(hint('settings.viewscreen.reset_all_hint'));
  resetSection.appendChild(hint('settings.viewscreen.reset_all_scope_hint'));
  const resetRow = row();
  resetRow.appendChild(control(
    VIEWSCREEN_PRESENTATION_CONTROLS.resetAll,
    'settings.viewscreen.reset_all',
    () => {
      presentation.resetAll();
      repaint();
    },
  ));
  resetSection.appendChild(resetRow);
  target.appendChild(resetSection);

  repaint();
  return { repaint };

  // ── Builders and painters ────────────────────────────────────────────────

  /** The live readout under one control: the value in force and where it came
   *  from. A polite live region, so pressing an option or dragging the slider
   *  ANNOUNCES the result rather than leaving a screen-reader operator to go
   *  looking for it. */
  function statusLine(controlId) {
    const el = doc.createElement('div');
    if (statusClass) el.className = statusClass;
    el.setAttribute('data-control', controlId);
    el.setAttribute('role', 'status');
    el.setAttribute('aria-live', 'polite');
    return el;
  }

  /** A per-setting reset: this effect follows the system again and no other
   *  value in the record is read or written (PRD #1418 story 17). */
  function resetButton(controlId, labelId, effect) {
    const wrapper = row();
    wrapper.appendChild(control(controlId, labelId, () => {
      presentation.reset(effect);
      repaint();
    }));
    return wrapper;
  }

  function statusText(entry, valueText) {
    return t('settings.viewscreen.status', {
      value: valueText,
      source: t(entry.available === false
        ? 'settings.viewscreen.source_unread'
        : SOURCE_LABELS[entry.source] || SOURCE_LABELS.default),
    });
  }

  function paintTextScale() {
    const status = presentation.status();
    const live = status.textScale.value;
    // The slider tracks the LIVE value, not the stored one: following a system
    // that asks for 150% must show 150% rather than 100% with the text already
    // enlarged around it.
    if (doc.activeElement !== slider) slider.value = String(live);
    const valueText = t('settings.viewscreen.text_scale_value', { value: percent(live) });
    readout.textContent = valueText;
    nodes.textStatus.textContent = statusText(status.textScale, valueText);
  }

  function paintContrast() {
    const status = presentation.status();
    const chosen = presentation.record().contrast;
    for (const [value] of CONTRAST_OPTIONS) {
      const el = nodes.contrast[value];
      if (!el) continue;
      const selected = chosen === value;
      el.classList.toggle('active', selected);
      el.setAttribute('aria-pressed', selected ? 'true' : 'false');
    }
    nodes.contrastStatus.textContent = statusText(
      status.contrast,
      t(status.contrast.value
        ? 'settings.viewscreen.contrast_more'
        : 'settings.viewscreen.contrast_standard'),
    );
  }

  function repaint() {
    paintTextScale();
    paintContrast();
  }
}
