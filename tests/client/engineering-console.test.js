// @vitest-environment jsdom
/**
 * tests/client/engineering-console.test.js — the two Engineering consoles on
 * one renderer (issue #1235, T4.C3 chunk 1).
 *
 * Each hull's `.html` imports its `renderStation` from
 * `gui/<class>/engineering.console.js`; this suite imports the SAME functions
 * and drives them against a jsdom fixture, so the power/repair/shields
 * contract is asserted per hull without a browser.
 *
 * The custom-element modules are deliberately NOT imported: an un-upgraded
 * `<ph-*>` element is a plain `HTMLUnknownElement`, so assigning `.state`
 * stores a readable property and we assert the exact object the console
 * pushed, with no shadow-DOM machinery in the way.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderStation as rawCruiserRender } from '../../gui/cruiser/engineering.console.js';
import { renderStation as rawDestroyerRender } from '../../gui/destroyer/engineering.console.js';
import { withConsoleFamilyProjection } from './console-family-fixture.js';

const cruiserRender = (payload, doc) => rawCruiserRender(withConsoleFamilyProjection(payload), doc);
const destroyerRender = (payload, doc) => rawDestroyerRender(withConsoleFamilyProjection(payload), doc);

function mount(markup) {
  document.body.innerHTML = markup;
}
const el = (id) => document.getElementById(id);

const CRUISER_FIXTURE =
  '<ph-power-controls id="power-controls"></ph-power-controls>' +
  '<ph-battery-bar id="battery-bar"></ph-battery-bar>' +
  '<ph-hull-integrity id="hull-integrity"></ph-hull-integrity>' +
  '<ph-station-damage id="core-damage"></ph-station-damage>' +
  '<ph-repair-teams id="repair-teams"></ph-repair-teams>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="engineering-auto-badge" hidden></span>';

const DESTROYER_FIXTURE =
  '<ph-shield-facings id="shield-facings"></ph-shield-facings>' +
  '<div id="threat-row"><span id="threat-bearing"></span></div>' +
  '<ph-power-controls id="power-controls"></ph-power-controls>' +
  '<ph-battery-bar id="battery-bar"></ph-battery-bar>' +
  '<ph-hull-integrity id="hull-integrity"></ph-hull-integrity>' +
  '<ph-station-damage id="core-damage"></ph-station-damage>' +
  '<ph-repair-teams id="repair-teams"></ph-repair-teams>' +
  '<ph-station-damage id="station-damage"></ph-station-damage>' +
  '<span id="engineering-auto-badge" hidden></span>' +
  '<div id="tractor-panel" hidden>' +
    '<button id="tractor-btn"></button><span id="tractor-status"></span>' +
    '<div id="tractor-refusal" hidden></div>' +
  '</div>' +
  '<div id="umbilical-panel" hidden>' +
    '<button id="umbilical-btn"></button><span id="umbilical-status"></span>' +
    '<div id="umbilical-refusal" hidden></div>' +
  '</div>' +
  '<div id="dispatch-panel" hidden>' +
    '<button id="dispatch-btn"></button><span id="dispatch-status"></span>' +
    '<div id="dispatch-refusal" hidden></div>' +
  '</div>';

describe('cruiser engineering renderStation', () => {
  beforeEach(() => mount(CRUISER_FIXTURE));

  const payload = {
    systems: {
      'power-reactor': { consoles: [{ id: 'p1' }], power_auto: true, battery_online: true, charging: true, battery_charge: 40, battery_max: 100 },
      repair: {
        overall_hull: { pct: 0.75, destroyed_pct: 0.1 },
        core_systems: [{ id: 'core-1' }],
        teams: [{ id: 't1' }],
        repair_auto: true,
        dispatch_targets: [{ id: 'dt1' }],
        damaged_systems: [{ id: 'ds1' }],
      },
    },
    own_hull: { pct: 0.9 },
  };

  it('drives power, battery, hull integrity and repair teams from projected families', () => {
    cruiserRender(payload, document);
    expect(el('power-controls').state).toEqual({ groups: [{ id: 'p1' }], auto: true });
    expect(el('battery-bar').state).toEqual({ level_pct: 40, charging: true, emergency_threshold_pct: 20 });
    expect(el('hull-integrity').state).toEqual({ total_pct: 0.75, destroyed_pct: 0.1 });
    expect(el('core-damage').state).toEqual({ entries: [{ id: 'core-1' }] });
    expect(el('repair-teams').state).toEqual({ teams: [{ id: 't1' }], auto: true, targets: [{ id: 'dt1' }], damaged: [{ id: 'ds1' }] });
    expect(el('station-damage').state).toEqual({ pct: 0.9 });
  });

  it('shows the AUTO badge only when power AND repair are both AI-run (no shields column to conjoin)', () => {
    cruiserRender(payload, document);
    expect(el('engineering-auto-badge').hidden).toBe(false);
    const halfAuto = {
      systems: {
        'power-reactor': { ...payload.systems['power-reactor'], power_auto: false },
        repair: payload.systems.repair,
      },
    };
    cruiserRender(halfAuto, document);
    expect(el('engineering-auto-badge').hidden).toBe(true);
  });

  it('carries no shields column and no tractor/umbilical/dispatch panels', () => {
    expect(el('shield-facings')).toBeNull();
    expect(el('tractor-panel')).toBeNull();
  });

  it('falls back to a full hull and zero battery when the payload carries nothing', () => {
    cruiserRender({ systems: {} }, document);
    expect(el('hull-integrity').state).toEqual({ total_pct: 1, destroyed_pct: undefined });
    expect(el('battery-bar').state.level_pct).toBe(0);
  });
});

describe('destroyer engineering renderStation', () => {
  beforeEach(() => mount(DESTROYER_FIXTURE));

  const basePayload = {
    systems: {
      'shields-system': { facings: [{ id: 'fwd' }], focused_facing: 'fwd', shields_auto: true, threat_bearing: 45.4 },
      'power-reactor': { consoles: [{ id: 'p1' }], power_auto: true, battery_online: true, charging: false, battery_charge: 60, battery_max: 100 },
      repair: {
        overall_hull: { pct: 1, destroyed_pct: 0 },
        core_systems: [],
        teams: [],
        repair_auto: true,
        dispatch_targets: [],
        damaged_systems: [],
      },
    },
    own_hull: { pct: 1 },
  };

  it('drives the shields column and threat-bearing readout', () => {
    destroyerRender(basePayload, document);
    expect(el('shield-facings').state).toEqual({ facings: [{ id: 'fwd' }], focused_facing: 'fwd', auto: true });
    expect(el('threat-row').classList.contains('active')).toBe(true);
    expect(el('threat-bearing').textContent).toBe('45°M');
  });

  it('clears the threat-bearing readout when Sensors holds no threat', () => {
    const noThreat = { ...basePayload, systems: { ...basePayload.systems, 'shields-system': { ...basePayload.systems['shields-system'], threat_bearing: null } } };
    destroyerRender(noThreat, document);
    expect(el('threat-row').classList.contains('active')).toBe(false);
    expect(el('threat-bearing').textContent).toBe('—');
  });

  it('conjoins shields_auto into the AUTO badge', () => {
    destroyerRender(basePayload, document);
    expect(el('engineering-auto-badge').hidden).toBe(false);
    const shieldsManual = { ...basePayload, systems: { ...basePayload.systems, 'shields-system': { ...basePayload.systems['shields-system'], shields_auto: false } } };
    destroyerRender(shieldsManual, document);
    expect(el('engineering-auto-badge').hidden).toBe(true);
  });

  it('hides the tractor/umbilical/dispatch panels when the hull carries no matching system', () => {
    destroyerRender(basePayload, document);
    expect(el('tractor-panel').hidden).toBe(true);
    expect(el('umbilical-panel').hidden).toBe(true);
    expect(el('dispatch-panel').hidden).toBe(true);
  });

  it('shows the tractor panel and toggles engage/release text+class', () => {
    const withTractor = { ...basePayload, systems: { ...basePayload.systems, tractor: { engaged: false, range: 500 } } };
    destroyerRender(withTractor, document);
    expect(el('tractor-panel').hidden).toBe(false);
    expect(el('tractor-btn').classList.contains('engaged')).toBe(false);
    expect(el('tractor-btn').textContent).toBe(t('console.tractor.engage'));

    const engaged = { ...basePayload, systems: { ...basePayload.systems, tractor: { engaged: true, coupled_target_name: 'console.tractor.idle' } } };
    destroyerRender(engaged, document);
    expect(el('tractor-btn').classList.contains('engaged')).toBe(true);
    expect(el('tractor-btn').textContent).toBe(t('console.tractor.release'));
  });

  it('shows a tractor refusal when present and clears it when absent', () => {
    const refused = { ...basePayload, systems: { ...basePayload.systems, tractor: { engaged: false, refusal: 'console.tractor.idle' } } };
    destroyerRender(refused, document);
    expect(el('tractor-refusal').hidden).toBe(false);
    const clear = { ...basePayload, systems: { ...basePayload.systems, tractor: { engaged: false } } };
    destroyerRender(clear, document);
    expect(el('tractor-refusal').hidden).toBe(true);
  });

  it('shows the umbilical panel and toggles start/stop text+class', () => {
    const idle = { ...basePayload, systems: { ...basePayload.systems, umbilical: { running: false, rate: 10, operator_level: 50, partner_level: null } } };
    destroyerRender(idle, document);
    expect(el('umbilical-panel').hidden).toBe(false);
    expect(el('umbilical-btn').classList.contains('engaged')).toBe(false);
    expect(el('umbilical-status').textContent).toContain('—');

    const running = { ...basePayload, systems: { ...basePayload.systems, umbilical: { running: true, rate: 10, operator_level: 50, partner_level: 30 } } };
    destroyerRender(running, document);
    expect(el('umbilical-btn').classList.contains('engaged')).toBe(true);
  });

  it('shows the external-dispatch panel and toggles dispatch/recall text+class', () => {
    const idle = { ...basePayload, systems: { ...basePayload.systems, repair: { ...basePayload.systems.repair, external_dispatch: { target: null, range: 200 } } } };
    destroyerRender(idle, document);
    expect(el('dispatch-panel').hidden).toBe(false);
    expect(el('dispatch-btn').classList.contains('engaged')).toBe(false);
    expect(el('dispatch-btn').textContent).toBe(t('console.repair.dispatch.send'));

    const working = { ...basePayload, systems: { ...basePayload.systems, repair: { ...basePayload.systems.repair, external_dispatch: { target: 'x', target_name: 'console.tractor.idle' } } } };
    destroyerRender(working, document);
    expect(el('dispatch-btn').classList.contains('engaged')).toBe(true);
    expect(el('dispatch-btn').textContent).toBe(t('console.repair.dispatch.recall'));
  });
});
