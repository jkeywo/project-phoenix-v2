// @vitest-environment jsdom

/**
 * tests/client/viewscreen-presentation-panel.test.js — the `phoneLimited`
 * option on gui/viewscreen-presentation-panel.js (issue #1429, PRD #1418
 * stories 18-19).
 *
 * "Phone Viewscreen settings expose text scaling for menus/overlays, contrast
 * and Reduce effects. Detailed individual effect controls are reserved for
 * larger surfaces." (PRD #1418, Implementation Decisions). This file proves
 * that sentence against the real render function: a phone-limited build
 * offers exactly text scale, contrast and the Reduce effects preset, never
 * the individual camera-shake/flash/decorative-motion rows or their "not
 * offered here" sentences (which exist only to explain an individual control
 * this build does not show); a full build keeps offering all of it, exactly
 * as issue #1428 left it; and Reduce effects still writes every effect the
 * surface renders regardless of which rows this panel chose to draw.
 */
import { describe, it, expect } from 'vitest';
import {
  renderViewscreenPresentationPanel,
  VIEWSCREEN_PRESENTATION_CONTROLS,
} from '../../gui/viewscreen-presentation-panel.js';
import { createViewscreenPresentation, FOLLOW_OS, EXPLICIT_ON } from '../../gui/viewscreen-presentation.js';
import { makeSectionBuilders, makeRowBuilder } from '../../gui/settings-overlay-kit.js';
import { t } from '../../gui/strings.js';

function control(id, labelId, onClick) {
  const el = document.createElement('button');
  el.type = 'button';
  el.setAttribute('data-control', id);
  el.textContent = t(labelId);
  el.addEventListener('click', (e) => {
    if (e && typeof e.preventDefault === 'function') e.preventDefault();
    onClick();
  });
  return el;
}

/** Build the panel into a fresh detached container and return it plus the
 *  live controller, so a test can both inspect the DOM and drive a click. */
function buildPanel(opts = {}) {
  const target = document.createElement('div');
  let stored = {};
  const presentation = createViewscreenPresentation({
    win: {},
    doc: document,
    store: {
      load: () => stored,
      save: (record) => { stored = record; },
    },
  });
  const { section, hint } = makeSectionBuilders(document, {
    sectionClass: 'sec', headingClass: 'head', hintClass: 'hint',
  });
  const row = makeRowBuilder(document, 'row');
  const result = renderViewscreenPresentationPanel(target, {
    doc: document,
    t,
    presentation,
    surface: opts.surface || 'viewscreen',
    phoneLimited: !!opts.phoneLimited,
    section,
    hint,
    row,
    control,
    classes: {},
  });
  return { target, presentation, repaint: result.repaint };
}

const byControl = (target, id) => target.querySelector(`[data-control="${id}"]`);

describe('renderViewscreenPresentationPanel phoneLimited', () => {
  it('offers text scale, contrast and Reduce effects — the three PRD #1418 names for a phone', () => {
    const { target } = buildPanel({ phoneLimited: true });
    expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.textScale)).toBeTruthy();
    expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.contrast(FOLLOW_OS))).toBeTruthy();
    expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.reduceEffects)).toBeTruthy();
  });

  it('omits every individual effect row and its "not offered here" sentence', () => {
    const { target } = buildPanel({ phoneLimited: true, surface: 'viewscreen' });
    for (const effect of ['shake', 'flash', 'decorativeMotion']) {
      expect(
        byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.effect(effect, 'full')),
        `${effect} row`,
      ).toBeFalsy();
      expect(
        byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.effectAbsent(effect)),
        `${effect} absent-sentence`,
      ).toBeFalsy();
    }
  });

  it('a full (non-phone) build keeps every individual effect control issue #1428 added', () => {
    const { target } = buildPanel({ phoneLimited: false, surface: 'viewscreen' });
    for (const effect of ['shake', 'flash', 'decorativeMotion']) {
      expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.effect(effect, 'full')), effect).toBeTruthy();
    }
  });

  it('a GM session omits the absent-shake/absent-flash sentences on phone too — nothing to explain when no row exists', () => {
    const { target } = buildPanel({ phoneLimited: true, surface: 'gm' });
    expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.effectAbsent('shake'))).toBeFalsy();
    expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.effectAbsent('flash'))).toBeFalsy();
    // decorativeMotion IS real on a GM session but still gets no individual row here.
    expect(byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.effect('decorativeMotion', 'full'))).toBeFalsy();
  });

  it('Reduce effects still writes every effect the surface renders, with no row to show it', () => {
    const { target, presentation } = buildPanel({ phoneLimited: true, surface: 'viewscreen' });
    byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.reduceEffects).click();
    const record = presentation.record();
    expect(record.shake).toBeCloseTo(0.3);
    expect(record.flash).toBeCloseTo(0.3);
    expect(record.decorativeMotion).toBeCloseTo(0.4);
  });

  it('text scale and contrast still work exactly as on a full Viewscreen', () => {
    const { target, presentation } = buildPanel({ phoneLimited: true });
    const slider = byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.textScale);
    slider.value = '2';
    slider.dispatchEvent(new window.Event('input', { bubbles: true }));
    expect(presentation.record().textScale).toBe(2);

    byControl(target, VIEWSCREEN_PRESENTATION_CONTROLS.contrast(EXPLICIT_ON)).click();
    expect(presentation.record().contrast).toBe(EXPLICIT_ON);
  });

  it('repaint() does not throw with the effect rows absent', () => {
    const { repaint } = buildPanel({ phoneLimited: true });
    expect(() => repaint()).not.toThrow();
  });
});
