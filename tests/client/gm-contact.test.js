// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { buildSensorsConsoleState, buildCommsConsoleState } from '../../gui/console-state.js';
import { crewContactRows } from '../../gui/gm-knowledge-compare.js';
import { createGmContactPanel } from '../../gui/gm-contact-panel.js';
import { createGmConfirmationController } from '../../gui/gm-confirmation.js';

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
    + '<select id="gm-contact-classification"></select><p id="gm-contact-classification-current"></p><button id="gm-contact-misclassify"></button><button id="gm-contact-classification-normal"></button>'
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
it('confirms the captured observer and target before starting the ordinary contact lifecycle', () => {
  let confirmation;
  const { panel, submit } = mount({ confirmAction: request => confirmation.request(request) });
  confirmation = createGmConfirmationController({ doc: document, profile: { mode: () => 'confirm' } });
  panel.choose('conceal');
  expect(panel.state().pending).toBeNull();
  document.querySelector('[data-confirmation-cancel]').click();
  expect(submit).not.toHaveBeenCalled();
  panel.choose('conceal');
  panel.update({ entities: [] });
  document.querySelector('[data-confirmation-accept]').click();
  expect(submit).toHaveBeenCalledExactlyOnceWith({ operator_id: 'gm', correlation: 'request',
    ship: 'observer', target: 'target', mode: 'conceal' });
  confirmation.destroy();
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

it('misclassification changes the selected observer sensor identity without mutating truth or other surfaces', () => {
  const input = state(null, 1000);
  const before = JSON.stringify(input);
  input.blackboards.sensors.contact_classifications = { target: 'Reported freighter' };
  const crew = JSON.parse(buildSensorsConsoleState(input));
  expect(crew.blips[0].name).toBe('Reported freighter');
  expect(crew.target_name).toBe('Reported freighter');
  expect(crew.target_class).toBe('Reported freighter');
  expect(crew.scan.reading.subject_name).toBe('SECRET-SCAN'); // independent earned reading
  expect(crewContactRows(crew.blips, input.asteroids)[0].name).toBe('Reported freighter');
  expect(input.asteroids[0]).toEqual(secret);
  expect(buildCommsConsoleState(input)).toBe(buildCommsConsoleState(JSON.parse(before)));
  expect(JSON.parse(buildSensorsConsoleState(JSON.parse(before))).target_name).toBe('SECRET-NAME');
});
it('classification never reveals a contact; Conceal wins and a revealed basic point gets only its false label', () => {
  for (const mode of [null, 'conceal', 'reveal']) {
    const input = state(mode); input.blackboards.scan.reading = null;
    input.blackboards.sensors.contact_classifications = { target: 'Reported freighter' };
    const crew = JSON.parse(buildSensorsConsoleState(input));
    if (mode !== 'reveal') expect(crew.blips).toEqual([]);
    else {
      expect(crew.blips[0]).toMatchObject({ name: 'Reported freighter', basic_contact: true });
      expect(crew.target_class).toBe('Reported freighter');
      expect(crew.target_hull_pct).toBeNull(); expect(crew.target_faction).toBeNull();
      expect(JSON.stringify(crew)).not.toContain('SECRET');
    }
  }
});
it('classification apply and clear use typed captured requests and exact correlated results', () => {
  const submitClassification = vi.fn(() => true);
  const { panel, entities } = mount({ submitClassification });
  const palette = [{ palette: 'freighter', label: 'Reported freighter' }];
  panel.update({ entities, contact_classification_palette: palette });
  const select = document.getElementById('gm-contact-classification');
  select.value = 'freighter'; select.dispatchEvent(new Event('change'));
  document.getElementById('gm-contact-misclassify').click();
  expect(submitClassification).toHaveBeenCalledExactlyOnceWith({ operator_id: 'gm', correlation: 'request', ship: 'observer', target: 'target', palette: 'freighter' });
  expect(panel.chooseClassification('freighter')).toBe(false);
  const result = { action_kind: 'contact-misclassify', observer: 'observer', target: 'target', operator_id: 'gm', correlation: 'request', tick: 42, outcome: 'applied' };
  panel.update({ entities, contact_classification_palette: palette, contact_results: [{ ...result, observer: 'other' }] });
  expect(panel.state().pending).not.toBeNull();
  panel.update({ entities, contact_classification_palette: palette, contact_classifications: { observer: { target: palette[0] } }, contact_results: [result] });
  expect(panel.state().pending).toBeNull();
  expect(document.getElementById('gm-contact-classification-current').textContent).toBe('Reported freighter');
  document.getElementById('gm-contact-classification-normal').click();
  expect(submitClassification).toHaveBeenLastCalledWith({ operator_id: 'gm', correlation: 'request', ship: 'observer', target: 'target', palette: null });
});
it('classification refuses unlisted choices, malformed projections and unavailable admission', () => {
  const submitClassification = vi.fn(() => true), { panel, entities, getOperator } = mount({ submitClassification });
  expect(panel.chooseClassification('unknown')).toBe(false);
  expect(panel.update({ entities, contact_classifications: { observer: { target: { label: 'bad' } } } })).toBe(false);
  panel.update({ entities, contact_classification_palette: [{ palette: 'freighter', label: 'Reported freighter' }] });
  getOperator.mockReturnValue(null); expect(panel.chooseClassification('freighter')).toBe(false);
  expect(submitClassification).not.toHaveBeenCalled(); panel.reset(); expect(panel.state().classifications).toEqual({});
});
it('live entity updates preserve the focused classification choice and its native option nodes', () => {
  const { panel, entities } = mount();
  const palette = [{ palette: 'freighter', label: 'Reported freighter' }, { palette: 'cruiser', label: 'Reported cruiser' }];
  panel.update({ entities, contact_classification_palette: palette });
  const select = document.getElementById('gm-contact-classification');
  select.focus(); select.value = 'cruiser';
  const options = [...select.options];
  for (let tick = 0; tick < 3; tick++) panel.update({
    entities: entities.map(row => ({ ...row, position: [tick, 0, 0] })),
    contact_classification_palette: palette.map(row => ({ ...row })),
  });
  expect(document.activeElement).toBe(select);
  expect(select.value).toBe('cruiser');
  options.forEach((option, i) => expect(select.options[i]).toBe(option));
  panel.update({ entities, contact_classification_palette: [palette[1]] });
  expect(select.value).toBe('cruiser');
  expect([...select.options].map(option => option.value)).toEqual(['', 'cruiser']);
  panel.update({ entities, contact_classification_palette: [palette[0]] });
  expect(select.value).toBe('');
});

it('shows an observer ghost as a basic untargetable crew-only report with no physical facts', () => {
  const input = state(null, 100);
  input.blackboards.sensors.contact_ghosts = [{ uuid: '__gm_ghost:observer:echo', name: 'Reported freighter', position: [200, 0, -20] }];
  input.sensorsTarget = '__gm_ghost:observer:echo';
  const output = JSON.parse(buildSensorsConsoleState(input));
  expect(output.blips).toHaveLength(1);
  expect(output.blips[0]).toMatchObject({ uuid: '__gm_ghost:observer:echo', name: 'Reported freighter', basic_contact: true, selectable: false, edge: true });
  expect(output.target_uuid).toBeNull();
  expect(output.target_alert).toBeNull(); expect(output.target_weapons).toBeNull(); expect(output.target_projection).toBeNull();
  expect(crewContactRows(output.blips, input.asteroids)).toEqual([{ id: '__gm_ghost:observer:echo', name: 'Reported freighter', hull_percent: null, destroyed: null }]);
  expect(JSON.parse(buildSensorsConsoleState(state(null, 100))).blips).toEqual([]);
});
it('lists each ghost an observing ship has been told, with its own Remove, and correlates it exactly', () => {
  // Placing a ghost is a Spawn outcome. What remains on the contact tool is the
  // record of what each observing ship has been told and the one simple action
  // that undoes it — a verb, not a draft.
  const submitInformation = vi.fn(() => true);
  const { panel, entities } = mount({ submitInformation });
  panel.select(null);
  document.body.insertAdjacentHTML('beforeend', '<ul id="gm-contact-ghosts"></ul>');
  const palette = [{ palette: 'freighter', label: 'Reported freighter' }];
  const ghost = { id: 'echo', palette: 'freighter', label: 'Reported freighter', position_mm: [1000, 0, -2000] };
  panel.update({ entities, contact_classification_palette: palette, contact_information: { ghosts: { observer: { echo: ghost } } } });
  const remove = document.querySelector('#gm-contact-ghosts button[data-role="remove"]');
  expect(remove.dataset.ghostId).toBe('echo');
  expect(document.querySelector('#gm-contact-ghosts li').textContent).toContain('echo: Reported freighter (1000, 0, -2000)');
  remove.focus();
  panel.update({ entities: entities.map(row => ({ ...row, pose: { x: 20 } })), contact_classification_palette: palette, contact_information: { ghosts: { observer: { echo: ghost } } } });
  expect(document.activeElement).toBe(remove); expect(document.querySelector('#gm-contact-ghosts button')).toBe(remove);
  // The tool composes no ghost of its own any more.
  expect(panel.chooseInformation({ set_ghost: { id: 'echo', palette: 'freighter', position_mm: [1000, 0, -2000] } })).toBe(false);
  expect(submitInformation).not.toHaveBeenCalled();
  remove.click();
  expect(submitInformation).toHaveBeenCalledExactlyOnceWith({ operator_id: 'gm', correlation: 'request', ship: 'observer', change: { remove_ghost: { id: 'echo' } } });
  const row = { operator_id: 'gm', correlation: 'request', observer: 'observer', target: 'echo', action_kind: 'contact-information', tick: 42, outcome: 'applied' };
  panel.update({ entities, contact_results: [{ ...row, observer: 'other' }] }); expect(panel.state().pending).not.toBeNull();
  panel.update({ entities, contact_results: [row] }); expect(panel.state().pending).toBeNull();
});

const report = { observed_tick: 10, age_ticks: 5, source: 'console.sensors.report_source', name: 'Observed freighter', position_mm: [10000, 0, -20000] };
it('uses only the reported position and captured identity across every live Sensors detail lane', () => {
  const input = state('reveal', 1000); input.blackboards.scan.reading = null;
  input.blackboards.sensors.contact_reports = { target: report };
  input.blackboards.sensors.contact_classifications = { target: 'NEW-CLASSIFICATION' };
  input.regions = [{ uuid: 'target', name: 'SECRET-REGION' }];
  input.navigationWaypoint = { source_uuid: 'target', x: 200, z: 0, label: 'SECRET-WAYPOINT' };
  input.blackboardKinds.weapons = 'Weapons'; input.blackboards.weapons = { target_uuid: 'target' };
  const raw = buildSensorsConsoleState(input), payload = JSON.parse(raw);
  expect(payload.target_name).toBe(report.name); expect(payload.target_report).toEqual(report);
  expect(payload.target_range).toBeCloseTo(Math.hypot(10, -20));
  expect(payload.blips.find(blip => blip.uuid === 'target')).toMatchObject({ basic_contact: true, report });
  expect(crewContactRows(payload.blips, input.asteroids)[0].report).toEqual(report);
  expect(raw).not.toMatch(/SECRET|NEW-CLASSIFICATION/); expect(payload.regions).toEqual([]);
  for (const key of ['target_kind', 'target_class', 'target_hull_pct', 'target_shield_freq', 'target_faction', 'target_alert', 'target_weapons', 'target_projection', 'target_threat']) expect(payload[key]).toBeNull();
});
it('withholds a pending observation despite Reveal and discards stale selection facts; Conceal wins over a released sample', () => {
  for (const [mode, sample] of [['reveal', null], ['conceal', report]]) {
    const input = state(mode, 1000); input.blackboards.scan.reading = null;
    input.blackboards.sensors.contact_reports = { target: sample };
    const payload = JSON.parse(buildSensorsConsoleState(input));
    expect(payload.blips).toEqual([]); expect(payload.target_uuid).toBeNull(); expect(payload.target_report).toBeNull();
    expect(payload.target_alert).toBeNull(); expect(payload.target_weapons).toBeNull();
  }
});
it('captures an explicit report policy before confirmation and correlates its terminal result', () => {
  let request; const submitInformation = vi.fn(() => true);
  const { panel, entities } = mount({ submitInformation, confirmAction: value => { request = value; return true; } });
  const change = { set_report_policy: { target: 'target', policy: { delay_ticks: 12, position_step_mm: 1000, hide_identity: true } } };
  expect(panel.chooseInformation(change)).toBe(true); change.set_report_policy.policy.delay_ticks = 99;
  request.accept(); expect(submitInformation.mock.calls[0][0].change.set_report_policy.policy.delay_ticks).toBe(12);
  panel.update({ entities, contact_results: [{ action_kind: 'contact-information', observer: 'other', target: 'target', operator_id: 'gm', correlation: 'request', tick: 42, outcome: 'applied' }] });
  expect(panel.state().pending).not.toBeNull();
  panel.update({ entities, contact_results: [{ action_kind: 'contact-information', observer: 'observer', target: 'target', operator_id: 'gm', correlation: 'request', tick: 42, outcome: 'applied' }] });
  expect(panel.state().pending).toBeNull();
  expect(panel.chooseInformation({ set_report_policy: { target: 'target', policy: { delay_ticks: -1, position_step_mm: 0, hide_identity: false } } })).toBe(false);
  expect(panel.update({ entities, contact_information: { ghosts: {}, reports: { observer: { target: {} } } } })).toBe(false);
});

it('holds independently earned scans off the manipulated Sensors lane and restores them when policy clears', () => {
  const input = state(null, 1000), reading = input.blackboards.scan.reading;
  input.blackboards.sensors.contact_reports = { target: report };
  expect(JSON.parse(buildSensorsConsoleState(input)).scan.reading).toBeNull();
  expect(input.blackboards.scan.reading).toBe(reading);
  input.blackboards.sensors.contact_reports = {};
  expect(JSON.parse(buildSensorsConsoleState(input)).scan.reading).toEqual(reading);
});

// ===== the three complex actions this tool composes (issue #1510) ==========
/** The whole surface: the contact tool and the three draft panels split out of
 * it, plus a stand-in for the chart the ghost picker arms. */
function mountDrafts(options = {}) {
  document.body.innerHTML = '<select id="gm-contact-observer"></select><p id="gm-contact-target"></p><p id="gm-contact-mode"></p>'
    + ['reveal', 'conceal', 'normal'].map(mode => `<button id="gm-contact-${mode}"></button>`).join('')
    + '<select id="gm-contact-classification"></select><p id="gm-contact-classification-current"></p>'
    + '<button id="gm-contact-misclassify"></button><button id="gm-contact-classification-normal"></button>'
    + '<input type="checkbox" id="gm-contact-misclassify-keep-open">'
    + '<p id="gm-contact-misclassify-scope"></p><p id="gm-contact-misclassify-feedback"></p>'
    + '<input id="gm-contact-report-delay" type="number" value="0"><input id="gm-contact-report-step" type="number" value="0">'
    + '<input id="gm-contact-report-identity" type="checkbox"><p id="gm-contact-report-current"></p>'
    + '<button id="gm-contact-report-set"></button><button id="gm-contact-report-clear"></button>'
    + '<input type="checkbox" id="gm-contact-report-keep-open">'
    + '<p id="gm-contact-report-scope"></p><p id="gm-contact-report-feedback"></p>'
    + '<ul id="gm-contact-ghosts"></ul>'
    + '<p id="gm-contact-feedback"></p><ol id="gm-contact-results"></ol>';
  const submitClassification = vi.fn(() => true), submitInformation = vi.fn(() => true);
  const onSucceeded = vi.fn();
  const panel = createGmContactPanel({ submitClassification, submitInformation, onSucceeded,
    getOperator: () => ({ id: 'gm' }), schedule: vi.fn(),
    correlation: () => 'request', ...options });
  const entities = [{ entity_id: 'observer', kind: 'player_ship', name: 'Observer' },
    { entity_id: 'target', kind: 'npc_ship', name: 'Target' }];
  const palette = [{ palette: 'freighter', label: 'Reported freighter' }];
  panel.update({ entities, contact_classification_palette: palette });
  panel.select(entities[1]);
  const select = document.getElementById('gm-contact-observer');
  select.value = 'observer'; select.dispatchEvent(new Event('change'));
  const settle = (action_kind, target, outcome = 'applied') => panel.update({ entities,
    contact_classification_palette: palette,
    contact_results: [{ action_kind, observer: 'observer', target, operator_id: 'gm',
      correlation: 'request', tick: 42, outcome }] });
  return { panel, entities, palette, onSucceeded, settle,
    submitClassification, submitInformation };
}

it('finishes each draft only on its own applied outcome, and never on a plain verb', () => {
  const { panel, palette, onSucceeded, settle } = mountDrafts();
  expect(panel.chooseClassification('freighter')).toBe(true);
  settle('contact-misclassify', 'target');
  expect(onSucceeded).toHaveBeenCalledExactlyOnceWith('misclassify');
  // A refusal leaves the draft standing: there is something to answer.
  expect(panel.chooseClassification('freighter')).toBe(true);
  settle('contact-misclassify', 'target', 'refused');
  expect(onSucceeded).toHaveBeenCalledOnce();
  // Normal classification, clearing a policy and removing a ghost each undo
  // something rather than compose it, so they finish no draft.
  expect(panel.chooseClassification(null)).toBe(true);
  settle('contact-classification-normal', 'target');
  expect(onSucceeded).toHaveBeenCalledOnce();
  expect(panel.chooseInformation({ set_report_policy: { target: 'target',
    policy: { delay_ticks: 12, position_step_mm: 0, hide_identity: false } } })).toBe(true);
  settle('contact-information', 'target');
  expect(onSucceeded).toHaveBeenLastCalledWith('report-policy');
  expect(panel.chooseInformation({ clear_report_policy: { target: 'target' } })).toBe(true);
  settle('contact-information', 'target');
  expect(onSucceeded).toHaveBeenCalledTimes(2);
  expect(panel.chooseInformation({ remove_ghost: { id: 'echo' } })).toBe(true);
  settle('contact-information', 'echo');
  expect(onSucceeded).toHaveBeenCalledTimes(2);
});

it('reports what each draft holds unsent, clears only that, and reads its own Keep open', () => {
  const { panel } = mountDrafts();
  const drafts = panel.drafts;
  for (const name of ['misclassify', 'report-policy']) {
    expect(drafts[name].isDirty()).toBe(false);
    expect(drafts[name].keepOpen()).toBe(false);
  }
  document.getElementById('gm-contact-classification').value = 'freighter';
  document.getElementById('gm-contact-report-delay').value = '12';
  expect(drafts.misclassify.isDirty()).toBe(true);
  expect(drafts['report-policy'].isDirty()).toBe(true);
  // Each clears its own fields and nothing else: two panels, two drafts.
  drafts['report-policy'].reset();
  expect(document.getElementById('gm-contact-report-delay').value).toBe('0');
  expect(document.getElementById('gm-contact-classification').value).toBe('freighter');
  drafts.misclassify.reset();
  expect(document.getElementById('gm-contact-classification').value).toBe('');
  document.getElementById('gm-contact-misclassify-keep-open').checked = true;
  expect(drafts.misclassify.keepOpen()).toBe(true);
  expect(drafts['report-policy'].keepOpen()).toBe(false);
  drafts.misclassify.focus();
  expect(document.activeElement).toBe(document.getElementById('gm-contact-classification'));
});

it('says how each draft went on the draft panel, not only on the contact tool', () => {
  // The three drafts are panels of their own, and a panel behind another tab
  // is `hidden`: a refusal written only into the contact tool is a refusal the
  // operator never sees.
  const { panel, settle } = mountDrafts({ submitInformation: () => false, submit: () => true });
  const feedback = draft => document.getElementById(`gm-contact-${draft}-feedback`);
  expect(panel.chooseClassification('freighter')).toBe(true);
  expect(feedback('misclassify').dataset.state).toBe('pending');
  settle('contact-misclassify', 'target', 'refused');
  expect(feedback('misclassify').dataset.state).toBe('refused');
  expect(document.getElementById('gm-contact-feedback').dataset.state).toBe('refused');
  // A verb belongs to the tool that carries it — and starting anything new
  // takes the last draft's news off the panel it was left on.
  expect(panel.choose('reveal')).toBe(true);
  expect(document.getElementById('gm-contact-feedback').dataset.state).toBe('pending');
  expect(feedback('misclassify').dataset.state).toBeUndefined();
  expect(feedback('misclassify').textContent).toBe('');
});

it('keeps what a landed draft described and drops which one it was about', () => {
  const { panel } = mountDrafts();
  const classification = document.getElementById('gm-contact-classification');
  classification.value = 'freighter';
  // The shared rule: after a success the choices that describe WHAT stay.
  panel.drafts.misclassify.reset({ keepReusable: true });
  expect(classification.value).toBe('freighter');
  // A fresh open starts empty, classification included.
  panel.drafts.misclassify.reset({ keepReusable: false });
  expect(classification.value).toBe('');
  // A chosen classification is a real choice, so closing the panel asks first.
  classification.value = 'freighter';
  expect(panel.drafts.misclassify.isDirty()).toBe(true);
});

it('takes every draft with the run it was for, open or closed', () => {
  const { panel } = mountDrafts();
  document.getElementById('gm-contact-classification').value = 'freighter';
  document.getElementById('gm-contact-report-delay').value = '12';
  // The run boundary reaches a draft nobody had OPEN, which no lifecycle
  // discard would ever touch: what was typed for a run that has gone goes.
  panel.reset();
  expect(document.getElementById('gm-contact-classification').value).toBe('');
  expect(document.getElementById('gm-contact-report-delay').value).toBe('0');
});
