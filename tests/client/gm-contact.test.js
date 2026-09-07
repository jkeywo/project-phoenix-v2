// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { buildSensorsConsoleState, buildCommsConsoleState } from '../../gui/console-state.js';
import { crewContactRows } from '../../gui/gm-knowledge-compare.js';
import { createGmContactPanel } from '../../gui/gm-contact-panel.js';

const secret = { uuid: 'target', x: 200, z: 0, name: 'SECRET-NAME', faction: 'SECRET-FACTION',
  radar_icon: 'ship', radar_size: 99, tags: ['ship'], shipClass: 'SECRET-CLASS', hull_pct: 10,
  hull_fraction: 0.1, shield_freq: 4, shields: [2], speed: 99, target_description: 'SECRET-DESCRIPTION' };
function state(mode, range = 100) {
  const result = { asteroids: [secret], shipX: 0, shipZ: 0, sensorsTarget: 'target',
    blackboards: { sensors: { kind: 'Sensors', data: { radar_range: range, radar_shows: [], radar_selects: ['ship'],
      contact_overrides: mode ? { target: mode } : {} } },
    sensor_radar: { kind: 'SensorRadar', data: { selected_target_alert: true, selected_target_weapons_cold: true, selected_target_relative_velocity: [20, 30] } },
    scan: { kind: 'Scan', data: { capable: true, reading: { subject_uuid: 'target', subject_name: 'SECRET-SCAN' } } } } };
  result.blackboardKinds = Object.fromEntries(Object.entries(result.blackboards).map(([key, value]) => [key, value.kind]));
  result.blackboards = Object.fromEntries(Object.entries(result.blackboards).map(([key, value]) => [key, value.data]));
  return result;
}
it('reveals an out-of-range basic contact without copying protected detail through any Sensors lane', () => {
  const input = state('reveal');
  input.blackboards.scan.reading = null;
  const raw = buildSensorsConsoleState(input);
  const payload = JSON.parse(raw);
  expect(payload.blips).toHaveLength(1);
  expect(payload.blips[0]).toMatchObject({ uuid: 'target', basic_contact: true, edge: true });
  expect(raw).not.toContain('SECRET');
  for (const key of ['target_kind', 'target_class', 'target_hull_pct', 'target_shield_freq', 'target_faction', 'target_alert', 'target_weapons', 'target_projection', 'target_threat']) expect(payload[key]).toBeNull();
  expect(payload.scan.reading).toBeNull();
  const knowledge = crewContactRows(payload.blips, input.asteroids);
  expect(knowledge[0]).toMatchObject({ hull_percent: null, destroyed: null });
});
it('conceals an ordinarily visible target and stale selection, scan, region and tactical overlay', () => {
  const input = state('conceal', 1000);
  input.regions = [{ uuid: 'target', name: 'SECRET-REGION' }];
  input.blackboardKinds.weapons = 'Weapons'; input.blackboards.weapons = { target_uuid: 'target' };
  const payload = JSON.parse(buildSensorsConsoleState(input));
  expect(payload.blips).toEqual([]); expect(payload.regions).toEqual([]);
  expect(payload.target_uuid).toBeNull(); expect(payload.scan.reading).toBeNull();
  expect(JSON.stringify(payload)).not.toContain('SECRET');
});
it('Normal restores the ordinary projection; observer B and independent Comms remain unchanged', () => {
  const other = state(null, 1000);
  const before = buildSensorsConsoleState(other);
  const input = state('conceal', 1000);
  expect(buildSensorsConsoleState(input)).not.toEqual(before);
  expect(buildSensorsConsoleState(other)).toEqual(before);
  expect(buildSensorsConsoleState(state('normal', 1000))).toEqual(before);
  expect(buildCommsConsoleState(input)).toEqual(buildCommsConsoleState(other));
});
function mount(options = {}) {
  document.body.innerHTML = '<select id="gm-contact-observer"></select><p id="gm-contact-target"></p><p id="gm-contact-mode"></p>'
    + ['reveal', 'conceal', 'normal'].map(mode => `<button id="gm-contact-${mode}"></button>`).join('')
    + '<p id="gm-contact-feedback"></p><ol id="gm-contact-results"></ol>';
  const submit = vi.fn(() => true), getOperator = vi.fn(() => ({ id: 'gm' }));
  const panel = createGmContactPanel({ submit, getOperator, schedule: vi.fn(), correlation: () => 'request', ...options });
  const entities = [{ entity_id: 'observer', kind: 'player_ship', name: 'Observer' }, { entity_id: 'target', kind: 'npc_ship', name: 'Target' }];
  panel.update({ entities }); panel.select(entities[1]);
  const select = document.getElementById('gm-contact-observer'); select.value = 'observer'; select.dispatchEvent(new Event('change'));
  return { panel, entities, submit, getOperator };
}
it.each(['reveal', 'conceal', 'normal'])('submits typed %s once and waits for its exact canonical result', mode => {
  const { panel, entities, submit } = mount();
  document.getElementById(`gm-contact-${mode}`).click();
  expect(submit).toHaveBeenCalledWith({ operator_id: 'gm', correlation: 'request', ship: 'observer', target: 'target', mode });
  expect(panel.choose(mode)).toBe(false);
  panel.update({ entities, contact_results: [{ action_kind: `contact-${mode}`, observer: 'observer', target: 'target', operator_id: 'gm', correlation: 'request', tick: 42, outcome: 'applied' }] });
  expect(panel.state().pending).toBeNull();
});
it('refuses stale target or observer, unadmitted operators and reset state', () => {
  for (const missing of ['target', 'observer']) {
    const { panel, entities, submit } = mount(); panel.update({ entities: entities.filter(row => row.entity_id !== missing) });
    expect(panel.choose('reveal')).toBe(false); expect(submit).not.toHaveBeenCalled();
  }
  const { panel, getOperator } = mount(); getOperator.mockReturnValue(null);
  expect(panel.choose('conceal')).toBe(false); panel.reset(); expect(panel.state().target).toBeNull();
});

it('Reveal is a visibility floor and preserves ordinary contact details and earned scans', () => {
  const normal = state(null, 1000), reveal = state('reveal', 1000);
  expect(buildSensorsConsoleState(reveal)).toEqual(buildSensorsConsoleState(normal));
  const out = JSON.parse(buildSensorsConsoleState(state('reveal')));
  expect(out.scan.reading.subject_name).toBe('SECRET-SCAN');
  expect(out.target_class).toBeNull();
});
it('an override preserves unrelated Objective region annotations byte for byte', () => {
  const input = state('conceal');
  const region = { uuid: 'objective-region', objective_target: true, name: 'Objective', half_extents: [7, 9], color: [1, 0, 0] };
  input.regions = [region];
  expect(JSON.parse(buildSensorsConsoleState(input)).regions).toEqual([region]);
});
it('all result outcomes come from the String Table', () => {
  const translate = vi.fn((id, params) => id === 'server.gm.contact.result' ? params.outcome : `translated:${id}`);
  const { panel, entities } = mount({ t: translate });
  panel.update({ entities, contact_results: ['applied', 'no-op', 'refused'].map((outcome, i) => ({
    action_kind: 'contact-normal', observer: 'observer', target: 'target', operator_id: 'gm', correlation: String(i), tick: i, outcome })) });
  expect([...document.getElementById('gm-contact-results').children].map(row => row.textContent)).toEqual(['translated:server.gm.contact.outcome_applied', 'translated:server.gm.contact.outcome_no_op', 'translated:server.gm.contact.outcome_refused']);
});
