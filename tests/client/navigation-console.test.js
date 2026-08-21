// @vitest-environment jsdom
/**
 * tests/client/navigation-console.test.js — the battleship's Navigation
 * console on the shared renderer (issue #1235, T4.C3 chunk 2).
 *
 * `gui/battleship/navigation.html` imports its `renderStation` from
 * `gui/battleship/navigation.console.js`; this suite imports the SAME
 * function and drives it against a jsdom fixture. Navigation became a
 * hero-bar station on every other hull (issues #1097/#1098), so this is the
 * only surviving variant.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation } from '../../gui/battleship/navigation.console.js';

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURE =
  '<ph-navigation-map id="navigation-map"></ph-navigation-map>' +
  '<ph-objective-list id="objective-list"></ph-objective-list>' +
  '<ph-civilian-traffic id="civilian-traffic"></ph-civilian-traffic>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="nav-contact-count">0</span>' +
  '<span id="waypoint-name"></span>' +
  '<span id="footer-target"></span>' +
  '<button id="btn-on-screen"></button>' +
  '<span id="navigation-auto-badge" hidden></span>';

describe('battleship navigation renderStation', () => {
  beforeEach(() => mount(FIXTURE));

  const payload = {
    blips: [{ id: 'b1' }, { id: 'b2' }],
    regions: [{ id: 'r1' }],
    radar_range: 8000,
    ship_x: 12, ship_z: -4, ship_heading: 270,
    waypoint: { name: 'Alpha Point' },
    objectives: [{ id: 'o1' }],
    civilians: [{ id: 'c1' }],
    own_hull: { pct: 1 },
    navigation_auto: false,
  };

  it('drives the chart, objective list and civilian traffic from the flat payload', () => {
    renderStation(payload, document);
    expect(el('navigation-map').state).toEqual({
      blips: [{ id: 'b1' }, { id: 'b2' }], regions: [{ id: 'r1' }], range: 8000,
      ship_pos: { x: 12, z: -4 }, ship_heading: 270, waypoint: { name: 'Alpha Point' },
    });
    expect(el('objective-list').state).toEqual({ objectives: [{ id: 'o1' }] });
    expect(el('civilian-traffic').state).toEqual({ civilians: [{ id: 'c1' }] });
    expect(el('station-damage').state).toEqual({ pct: 1 });
    expect(el('nav-contact-count').textContent).toBe('2');
  });

  it('shows the waypoint name in both the side panel and the footer', () => {
    renderStation(payload, document);
    expect(el('waypoint-name').textContent).toBe('Alpha Point');
    expect(el('footer-target').textContent).toBe('Alpha Point');
  });

  it('falls back to a localized placeholder for an unnamed waypoint and for none set', () => {
    renderStation({ ...payload, waypoint: { name: '' } }, document);
    expect(el('waypoint-name').textContent).toBe(t('console.common.waypoint'));
    renderStation({ ...payload, waypoint: null }, document);
    expect(el('waypoint-name').textContent).toBe(t('console.navigation.not_set'));
  });

  it('disables the On-Screen button and shows the AUTO badge only when navigation is AI-run', () => {
    renderStation(payload, document);
    expect(el('btn-on-screen').disabled).toBe(false);
    expect(el('navigation-auto-badge').hidden).toBe(true);
    renderStation({ ...payload, navigation_auto: true }, document);
    expect(el('btn-on-screen').disabled).toBe(true);
    expect(el('navigation-auto-badge').hidden).toBe(false);
  });
});
