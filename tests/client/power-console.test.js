// @vitest-environment jsdom
/**
 * tests/client/power-console.test.js — the battleship's Power console on the
 * shared renderer (issue #1235, T4.C3 chunk 1).
 *
 * `gui/battleship/power.html` imports `renderStation` from
 * `gui/battleship/power.console.js`; this suite imports the SAME function and
 * drives it against a jsdom fixture, so the contract is asserted without a
 * browser. Only the battleship stations a dedicated Power console today (the
 * other hulls fold power into Engineering — see engineering-console.test.js),
 * so this suite is single-hull, matching gui/stations/power-console.js's own
 * "one hull today, N later" framing.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { renderStation } from '../../gui/battleship/power.console.js';

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURE =
  '<div id="power-data" hidden></div>' +
  '<ph-power-controls id="power-controls"></ph-power-controls>' +
  '<ph-battery-bar id="battery-bar"></ph-battery-bar>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="power-auto-badge" hidden></span>' +
  '<span id="bat-val">0%</span>';

describe('battleship power renderStation', () => {
  beforeEach(() => mount(FIXTURE));

  const payload = {
    groups: [{ id: 'g1', level: 2, commanded_level: 3, max_level: 4 }],
    power_auto: false,
    charging: true,
    battery_charge: 30,
    battery_max: 100,
    own_hull: { pct: 0.6 },
    total: 12,
    total_max: 20,
    draining: true,
  };

  it('drives the controls, battery, station-damage and footer label', () => {
    renderStation(payload, document);
    expect(el('power-controls').state).toEqual({ groups: payload.groups, auto: false });
    expect(el('battery-bar').state).toEqual({ level_pct: 30, charging: true, emergency_threshold_pct: 20 });
    expect(el('station-damage').state).toEqual({ pct: 0.6 });
    expect(el('bat-val').textContent).toBe('30%');
  });

  it('reflects the AUTO badge', () => {
    renderStation(payload, document);
    expect(el('power-auto-badge').hidden).toBe(true);
    renderStation({ ...payload, power_auto: true }, document);
    expect(el('power-auto-badge').hidden).toBe(false);
  });

  it('mirrors group data onto the hidden #power-data element for the smoke spec', () => {
    renderStation(payload, document);
    const dataEl = el('power-data');
    expect(dataEl.dataset.total).toBe('12');
    expect(dataEl.dataset.totalMax).toBe('20');
    expect(dataEl.dataset.draining).toBe('true');
    expect(dataEl.children.length).toBe(1);
    const entry = dataEl.children[0];
    expect(entry.dataset.id).toBe('g1');
    expect(entry.dataset.level).toBe('2');
    expect(entry.dataset.commandedLevel).toBe('3');
    expect(entry.dataset.maxLevel).toBe('4');
  });

  it('falls back to a zero battery reading when the wire sends no max', () => {
    renderStation({ ...payload, battery_max: 0 }, document);
    expect(el('battery-bar').state.level_pct).toBe(0);
  });

  it('accepts `consoles` as a `groups` fallback key', () => {
    const { groups, ...rest } = payload;
    renderStation({ ...rest, consoles: groups }, document);
    expect(el('power-controls').state.groups).toEqual(groups);
  });
});
