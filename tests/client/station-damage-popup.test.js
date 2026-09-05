// @vitest-environment jsdom
/**
 * tests/client/station-damage-popup.test.js — the Station Bar's per-system
 * damage popup (gui/station-damage-popup.js, issue #1374).
 *
 * The surface the console footers' `ph-station-damage` bar used to be. What
 * moved is the PLACE, not the information: the same `own_hull` rows, now
 * reached by tapping the already-selected Station tab instead of a strip at
 * the bottom of twenty-two different consoles.
 *
 * These drive the module through its public functions against the same markup
 * client.html mounts, so they hold the two halves that actually break: what
 * the popup SAYS for a Station (including the Station that owns no damageable
 * systems at all — a Captain's chair), and that it behaves as a layer a
 * keyboard can get into and back out of.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import {
  closeStationDamagePopup, isStationDamagePopupOpen, openStationDamagePopup,
  renderStationDamagePopup, stationDamagePopupModel,
} from '../../gui/station-damage-popup.js';

/**
 * client.html's popup markup, minus the styling. `ph-damage-detail` is a real
 * custom element in the browser; here a plain div stands in, because what this
 * module owes it is a `.state` assignment and a `hidden` — the drawing is
 * ph-damage-detail's own suite.
 *
 * Note what the stand-in means for the `hidden` assertions below: they pin
 * that this module SETS the attribute, not that the attribute hides anything.
 * A shadow host whose `:host` declares a display needs its own
 * `:host([hidden])` rule or the attribute is inert, and neither a plain div
 * nor jsdom (which does not cascade shadow styles at all) can see that. That
 * half is held on the component that owes it, in
 * tests/client/ph-damage-detail.test.js.
 */
const MARKUP = `
  <button id="opener" type="button">Tactical</button>
  <div id="station-damage-popup" hidden>
    <div class="popup-card" aria-labelledby="station-damage-popup-title">
      <div class="popup-head">
        <span id="station-damage-popup-title"></span>
        <button type="button" id="station-damage-popup-close">x</button>
      </div>
      <div id="station-damage-popup-detail"></div>
      <div id="station-damage-popup-empty" hidden>No damage model</div>
    </div>
  </div>
`;

const ENTRIES = [
  { display_name: 'Phaser Array', current: 40, max_hp: 80, tier: 2 },
  { display_name: 'Torpedo Bay', current: 0, max_hp: 60, tier: 1 },
];

const el = (id) => document.getElementById(id);
const popup = () => el('station-damage-popup');

beforeEach(() => {
  document.body.innerHTML = MARKUP;
});

describe('what the popup says', () => {
  it('titles itself with the Station the shell named', () => {
    const model = stationDamagePopupModel({ station: 'Tactical', entries: ENTRIES });
    expect(model.title)
      .toBe(t('component.station_damage.popup_title', { name: 'Tactical' }));
    expect(model.empty).toBe(false);
    expect(model.entries).toEqual(ENTRIES);
  });

  it('reports a Station with no damageable systems as empty, not as broken', () => {
    // A Captain's chair owns no hull of its own; so does a console whose first
    // `console_hull` post has not arrived yet.
    expect(stationDamagePopupModel({ station: 'Captain', entries: [] }).empty).toBe(true);
    expect(stationDamagePopupModel({ station: 'Captain' }).empty).toBe(true);
    expect(stationDamagePopupModel().empty).toBe(true);
  });

  it('drops holes in the row list rather than handing them to the detail', () => {
    const model = stationDamagePopupModel({ entries: [ENTRIES[0], null, undefined] });
    expect(model.entries).toEqual([ENTRIES[0]]);
  });
});

describe('painting it', () => {
  it('writes the title and hands the rows to the detail element', () => {
    const model = renderStationDamagePopup(document, { station: 'Helm', entries: ENTRIES });
    expect(model).not.toBeNull();
    expect(el('station-damage-popup-title').textContent)
      .toBe(t('component.station_damage.popup_title', { name: 'Helm' }));
    expect(el('station-damage-popup-detail').state).toEqual({ entries: ENTRIES });
    expect(el('station-damage-popup-detail').hidden).toBe(false);
    expect(el('station-damage-popup-empty').hidden).toBe(true);
  });

  it('shows the no-damage-model line instead of an empty box', () => {
    renderStationDamagePopup(document, { station: 'Captain', entries: [] });
    expect(el('station-damage-popup-detail').hidden).toBe(true);
    expect(el('station-damage-popup-empty').hidden).toBe(false);
  });

  it('repaints in place, so an open popup follows a system taking damage', () => {
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    const hit = [{ display_name: 'Phaser Array', current: 5, max_hp: 80, tier: 2 }];
    renderStationDamagePopup(document, { station: 'Tactical', entries: hit });
    expect(el('station-damage-popup-detail').state).toEqual({ entries: hit });
    expect(isStationDamagePopupOpen(document)).toBe(true);
  });

  it('returns null rather than throwing where the popup is not mounted', () => {
    document.body.innerHTML = '';
    expect(renderStationDamagePopup(document, { station: 'Helm' })).toBeNull();
    expect(openStationDamagePopup(document, { station: 'Helm' })).toBeNull();
    expect(isStationDamagePopupOpen(document)).toBe(false);
  });
});

describe('opening and closing it', () => {
  it('reveals the layer and takes focus into it', () => {
    el('opener').focus();
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    expect(isStationDamagePopupOpen(document)).toBe(true);
    expect(document.activeElement).toBe(el('station-damage-popup-close'));
  });

  it('declares itself a dialog beside the trap that makes that true', () => {
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    const card = popup().querySelector('.popup-card');
    expect(card.getAttribute('role')).toBe('dialog');
    expect(card.getAttribute('aria-modal')).toBe('true');
  });

  it('hands focus back to whatever opened it', () => {
    el('opener').focus();
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    closeStationDamagePopup(document);
    expect(isStationDamagePopupOpen(document)).toBe(false);
    expect(document.activeElement).toBe(el('opener'));
  });

  it('closes on the close button', () => {
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    el('station-damage-popup-close').click();
    expect(isStationDamagePopupOpen(document)).toBe(false);
  });

  it('closes on Escape, so a keyboard is never stranded inside it', () => {
    el('opener').focus();
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(isStationDamagePopupOpen(document)).toBe(false);
    expect(document.activeElement).toBe(el('opener'));
  });

  it('closes on the scrim but NOT on the card', () => {
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    // A tap that lands on a system row must not also be a dismiss.
    popup().querySelector('.popup-card').dispatchEvent(
      new window.MouseEvent('click', { bubbles: true }),
    );
    expect(isStationDamagePopupOpen(document)).toBe(true);
    popup().dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    expect(isStationDamagePopupOpen(document)).toBe(false);
  });

  it('closing twice is a no-op, not a second focus jump', () => {
    el('opener').focus();
    openStationDamagePopup(document, { station: 'Tactical', entries: ENTRIES });
    closeStationDamagePopup(document);
    el('opener').blur();
    closeStationDamagePopup(document);
    expect(document.activeElement).not.toBe(el('station-damage-popup-close'));
  });
});
