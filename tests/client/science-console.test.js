// @vitest-environment jsdom
/**
 * tests/client/science-console.test.js — the cruiser's Science console on
 * the shared renderer (issue #1235, T4.C3 chunk 2).
 *
 * `gui/cruiser/science.html` imports its `renderStation` from
 * `gui/cruiser/science.console.js`; this suite imports the SAME function and
 * drives it against a jsdom fixture. Science is a system-id-KEYED
 * `SystemStationConsolePayload` (no single `family`), read through
 * projected Console Family views — this is the only surviving variant.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as rawRenderStation } from '../../gui/cruiser/science.console.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';

const renderStation = (payload, doc) => rawRenderStation(withConsoleFamilyProjection(payload), doc);

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const FIXTURE =
  '<ph-sensor-radar id="sensor-radar"></ph-sensor-radar>' +
  '<ph-sensor-panel id="sensor-panel"></ph-sensor-panel>' +
  '<ph-shield-panel id="shield-panel"></ph-shield-panel>' +
  '<ph-shield-facings id="shield-facings"></ph-shield-facings>' +
  '<div id="threat-row"><span id="threat-bearing"></span></div>' +
  '<span id="footer-target"></span>' +
  '<span id="science-auto-badge" hidden></span>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>';

describe('cruiser science renderStation', () => {
  beforeEach(() => mount(FIXTURE));

  const payload = {
    systems: {
      sensors: { blips: [{ id: 'b1' }], target_name: 'Raider', sensors_auto: true },
      'shields-system': { facings: [{ id: 'fwd' }], focused_facing: 'fwd', shields_auto: true, threat_bearing: 88.6 },
    },
    own_hull: { pct: 0.6 },
  };

  it('reads sensors and shields through projected families and drives both radar+panel from the sensors view', () => {
    renderStation(payload, document);
    expect(el('sensor-radar').state).toEqual(payload.systems.sensors);
    expect(el('sensor-panel').state).toEqual(payload.systems.sensors);
    expect(el('shield-panel').state).toEqual(payload.systems['shields-system']);
    expect(el('shield-facings').state).toEqual({ facings: [{ id: 'fwd' }], focused_facing: 'fwd', auto: true });
    expect(el('station-damage').state).toEqual({ pct: 0.6 });
  });

  it('shows the active threat-bearing readout and clears it when Sensors holds no threat', () => {
    renderStation(payload, document);
    expect(el('threat-row').classList.contains('active')).toBe(true);
    expect(el('threat-bearing').textContent).toBe('89°M');
    const noThreat = { ...payload, systems: { ...payload.systems, 'shields-system': { ...payload.systems['shields-system'], threat_bearing: null } } };
    renderStation(noThreat, document);
    expect(el('threat-row').classList.contains('active')).toBe(false);
    expect(el('threat-bearing').textContent).toBe('—');
  });

  it('shows the glyph-prefixed, tinted target footer when Sensors holds a lock', () => {
    renderStation(payload, document);
    expect(el('footer-target').textContent).toBe('◉ Raider');
    expect(el('footer-target').style.color).toBe('var(--tactical)');
    const noTarget = { ...payload, systems: { ...payload.systems, sensors: { ...payload.systems.sensors, target_name: null } } };
    renderStation(noTarget, document);
    expect(el('footer-target').textContent).toBe(t('console.common.no_target'));
    expect(el('footer-target').style.color).toBe('var(--ink-faint)');
  });

  it('shows the AUTO badge only when sensors AND shields are both AI-run', () => {
    renderStation(payload, document);
    expect(el('science-auto-badge').hidden).toBe(false);
    const halfAuto = { ...payload, systems: { ...payload.systems, sensors: { ...payload.systems.sensors, sensors_auto: false } } };
    renderStation(halfAuto, document);
    expect(el('science-auto-badge').hidden).toBe(true);
  });
});
