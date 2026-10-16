/** @vitest-environment jsdom */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildSensorsConsoleState, buildWeaponsConsoleState, buildHelmConsoleState, buildTargetBlip } from '../../gui/console-state.js';
import { consoleReadState } from '../../gui/console-topology.js';
import { projectSensorTracks, sensorTrackText } from '../../gui/sensor-tracks.js';
import { createSensorScienceActionRegistry, SENSORS_TARGET_ACTION_ID, SENSORS_SCAN_ACTION_ID } from '../../gui/stations/sensors-actions.js';
import '../../gui/components/ph-sensor-panel.js';
import { t } from '../../gui/strings.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';

const sample = (extra = {}) => ({ contact_id: 'contact-1', selectable: true, observed_tick: 10, position: [10, -20],
  position_error: 4, stage: 'return', predicted: false, ...extra });
function raw(tracks = [sample()]) {
  return { blackboards: { broad: { tracks, selected_target: 'contact-1' }, 'scan/5:focus': { owner: 'focus', capable: true },
    sensors: { radar_range: 100 } }, blackboardKinds: { broad: 'SensorRadar', 'scan/5:focus': 'Scan', sensors: 'Sensors' },
    observationTick: 30, sensorsTarget: 'secret-uuid', controlSources: { focus: 'Human' },
    asteroids: [{ uuid: 'secret-uuid', name: 'SECRET NAME', shipClass: 'SECRET CLASS', position: [2, 0, 3],
      radar_icon: 'battleship', radar_size: 999, faction: 'SECRET FACTION', shield_frequency: .9 }],
    regions: [{ uuid: 'secret-region', name: 'SECRET REGION' }], shipX: 0, shipZ: 0, shipYaw: 0 };
}
beforeEach(() => { document.body.innerHTML = ''; window.activateSemanticAction = vi.fn(); });
describe('permitted broad track radar', () => {
  it('combines independent already-permitted SensorRadar records without a private identity join', () => {
    const state = raw([]);
    state.blackboards.second = { tracks: [sample({ contact_id: 'second-observation' })], selected_target: 'second-observation' };
    state.blackboards.third = { tracks: [sample({ contact_id: 'third-observation', position: [30, -40] })], selected_target: null };
    state.blackboardKinds.second = 'SensorRadar'; state.blackboardKinds.third = 'SensorRadar';
    const view = buildSensorsConsoleState(state, ['broad', 'second', 'third']);
    expect(view.tracks.map(track => track.id)).toEqual(['second-observation', 'third-observation']);
    expect(view.target_uuid).toBe('second-observation');
    expect(JSON.stringify(view)).not.toContain('secret-uuid');
  });
  it('shows reported synthetic evidence through measured tracks without a second ghost list or a physical target', () => {
    const contact = sample({ position: [1000, 0], selectable: false, stage: 'identified',
      identity: { name: 'Reported contact', faction: null }, identity_tick: 10,
      observation: { subject: { kind: 'entity', uuid: 'contact-1' }, observed_tick: 10,
        provenance: { kind: 'sensors_report', system: 'broad', label: 'console.sensors.report_source' } } });
    const state = raw([contact]); state.blackboards.broad.selected_target = null;
    state.blackboards.sensors.contact_ghosts = [{ uuid: '__gm_ghost:SECRET', name: 'DO NOT COPY', position: [2, 0, 3] }];
    const view = buildSensorsConsoleState(state, ['focus', 'broad']);
    expect(view.blips).toHaveLength(1); expect(view.tracks).toHaveLength(1);
    expect(view.blips[0]).toMatchObject({ uuid: 'contact-1', name: 'Reported contact', radar_x: .96, radar_y: 0, edge: true, selectable: false });
    expect(view.tracks[0]).toMatchObject({ age: 20, selectable: false });
    expect(view.target_uuid).toBeNull(); expect(contact.position).toEqual([1000, 0]);
    expect(JSON.stringify(view)).not.toMatch(/SECRET|DO NOT COPY|__gm_ghost/);
    const legacy = buildTargetBlip('reported', [{ uuid: 'reported', position: [1000, 0, 0] }], 0, 0, 0, 100, { edgeClamp: true });
    expect([view.blips[0].radar_x, view.blips[0].radar_y]).toEqual([legacy.radar_x, legacy.radar_y]);
    expect(projectSensorTracks([{ ...contact, observation: undefined }], { range: 100 }).blips).toEqual([]);
  });
  it('shows original received sample provenance without counting relays as new sources', () => {
    const origin = { source: 'radar-origin-0000000000000001', system: 'sensor-a' };
    const copy = { origin, sample_id: 'original-sample', observed_tick: 7, received_tick: 10 };
    const view = projectSensorTracks([sample({ samples: [copy, { ...copy, received_tick: 11 }, {
      origin: { source: 'radar-origin-0000000000000002', system: 'sensor-b' },
      sample_id: 'independent-sample', observed_tick: 7, received_tick: 10,
    }] })], { range: 100, tick: 30 });
    const row = view.tracks[0];
    expect(row).toMatchObject({ received: true, source_count: 2, age: 20 });
    expect(row.source_details[0]).toBe(t('sensing.sharing.origin', { source: origin.source, instrument: origin.system, tick: 7 }));
    expect(sensorTrackText(row)).toContain(t('sensing.sharing.received', { count: 2 }));
  });
  it('uses measured contact IDs and circles without entity identity or hull geometry fallback', () => {
    const view = buildSensorsConsoleState(raw(), ['focus', 'broad']);
    expect(view.blips).toHaveLength(1); expect(view.blips[0]).toMatchObject({ uuid: 'contact-1', radar_x: .1,
      radar_y: .2, uncertainty_radius: .04, scaled_radius: 0, icon: null, selectable: true });
    expect(view.target_name).toBe(t('console.sensors.basic_contact')); expect(view.target_class).toBeNull();
    expect(view.target_hull_pct).toBeNull(); expect(view.target_speed).toBeNull(); expect(view.target_projection).toBeNull();
    expect(view.regions).toEqual([]); expect(JSON.stringify(view)).not.toContain('SECRET'); expect(JSON.stringify(view)).not.toContain('secret-uuid');
    const empty = buildSensorsConsoleState(raw([]), ['broad']); expect(empty.blips).toEqual([]); expect(empty.target_uuid).toBeNull();
    const cleared = raw(); cleared.sensorsTarget = 'contact-1'; cleared.blackboards.broad.selected_target = null;
    expect(buildSensorsConsoleState(cleared, ['broad']).target_uuid).toBeNull();
  });
  it('rotates actual delivered positions and shows retained facts with their own ages', () => {
    const view = projectSensorTracks([sample({ stage: 'assessed', position: [20, 0], velocity: [3, -4],
      identity: { name: 'Known literal', faction: 'Known faction' }, identity_tick: 8, classification: 'Measured class',
      classification_tick: 9, assessment: { alert: false, weapons_restrained: true, disabled: true }, assessment_tick: 10 })],
    { range: 100, yaw: Math.PI / 2, selected: 'contact-1', tick: 30 });
    expect(view.blips[0].radar_x).toBeCloseTo(0); expect(view.blips[0].radar_y).toBeCloseTo(.2);
    expect(view.target_name).toBe('Known literal'); expect(view.target_speed).toBe(5); expect(view.target_restrained).toBe(true);
    expect(view.target_weapons).toBeNull(); expect(view.target_track.fact_ages.map(row => row.ticks)).toEqual([22, 21, 20]);
  });
  it('cannot read a cohosted unpermitted track board', () => {
    const state = raw(); state.blackboards.hidden = { tracks: [sample({ contact_id: 'hidden-contact' })] }; state.blackboardKinds.hidden = 'SensorRadar';
    const read = consoleReadState(state, { id: 'science', systems: ['focus'], feeds: [{ source: 'broad', kind: 'SensorRadar' }, { source: 'scan', kind: 'Scan' }] });
    expect(buildSensorsConsoleState(read, ['focus']).tracks.map(row => row.id)).toEqual(['contact-1']);
  });
  it('keeps native list focus while uncertainty ages; predicted returns cannot send target or scan', () => {
    const state = raw(); let view = buildSensorsConsoleState(state, ['focus', 'broad']);
    document.body.innerHTML = '<ph-sensor-panel></ph-sensor-panel>'; const panel = document.querySelector('ph-sensor-panel'); panel.state = view;
    const button = panel.shadowRoot.querySelector('#tracks button'); button.focus(); button.click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(SENSORS_TARGET_ACTION_ID, { source: 'control', detail: { uuid: 'contact-1' } });
    const key = vi.fn(); document.addEventListener('keydown', key);
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'N', code: 'KeyN', shiftKey: true, bubbles: true, composed: true }));
    expect(key).not.toHaveBeenCalled(); document.removeEventListener('keydown', key);
    view = buildSensorsConsoleState({ ...state, observationTick: 40 }, ['focus', 'broad']); panel.state = view;
    expect(panel.shadowRoot.activeElement).toBe(button); expect(button.textContent).toContain(t('observation.ticks', { ticks: 30 }));
    const send = vi.fn(); let n = 0; const feedback = new ActionFeedbackLifecycle({ correlation: () => `track-${++n}` });
    const registry = createSensorScienceActionRegistry({ getState: () => view, sendAction: send, actionFeedback: feedback });
    registry.activate(SENSORS_TARGET_ACTION_ID, { context: 'sensors', detail: { uuid: 'contact-1' } }); expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenLastCalledWith('set_sensors_target', expect.objectContaining({ uuid: 'contact-1', correlation: expect.any(String) }));
    feedback.settle(send.mock.calls[0][1].correlation, 'Refused'); expect(feedback.get(send.mock.calls[0][1].correlation).state).toBe('Refused');
    view = buildSensorsConsoleState(raw([sample({ predicted: true, position_error: 8 })]), ['focus', 'broad']); panel.state = view;
    expect(button.disabled).toBe(true); expect(button.textContent).toContain(t('sensing.track.predicted'));
    registry.activate(SENSORS_TARGET_ACTION_ID, { context: 'sensors', detail: { uuid: 'contact-1' } });
    registry.activate(SENSORS_SCAN_ACTION_ID, { context: 'sensors' }); expect(send).toHaveBeenCalledTimes(1);
    view = buildSensorsConsoleState(raw([]), ['focus', 'broad']); panel.state = view; expect(panel.shadowRoot.querySelector('#tracks button')).toBeNull();
  });
  it('rejects malformed records without accidentally selecting an earlier malformed duplicate', () => {
    const view = projectSensorTracks([sample({ position: [NaN, 0] }), sample()], { range: 100, selected: 'contact-1' });
    expect(view.target_range).toBeCloseTo(Math.hypot(10, 20)); expect(view.tracks).toHaveLength(1);
    expect(projectSensorTracks([sample({ position_error: -1 })], { range: 100 }).blips).toEqual([]);
  });
  it('requires explicit measured targeting permission even for a current identified return', () => {
    for (const selectable of [false, undefined]) {
      const view = projectSensorTracks([sample({ selectable, stage: 'identified', identity: { name: 'Known' } })], { range: 100 });
      expect(view.blips).toHaveLength(1);
      expect(view.blips[0].selectable).toBe(false);
      expect(view.tracks[0].selectable).toBe(false);
    }
  });
  it('keeps Tactical empty when the actual measured picture is empty instead of restoring raw world truth', () => {
    const state = raw();
    state.blackboardKinds.tactical = 'TacticalRadar';
    state.blackboards.tactical = { tracks: [], selected_target: null };
    state.weaponsBlips = [{ uuid: 'secret-uuid', name: 'SECRET NAME' }];
    state.weaponsTarget = 'secret-uuid'; state.weaponsTargetName = 'SECRET NAME';
    const view = buildWeaponsConsoleState(state, ['tactical']);
    expect(view.blips).toEqual([]); expect(view.target_uuid).toBeNull();
    expect(view.target_name).toBeNull(); expect(view.target_hull_pct).toBeNull();
    expect(JSON.stringify(view)).not.toContain('SECRET');
  });
  it('shows Tactical uncertainty and only measured target facts', () => {
    const state = raw(); state.blackboardKinds.tactical = 'TacticalRadar';
    state.blackboards.tactical = { tracks: [sample()], selected_target: 'contact-1' };
    state.weaponsRadarRange = 100;
    const view = buildWeaponsConsoleState(state, ['tactical']);
    expect(view.blips[0]).toMatchObject({ uuid: 'contact-1', uncertainty_radius: .04 });
    expect(view.target_name).toBe(t('console.sensors.basic_contact'));
    expect(view.target_hull_pct).toBeNull(); expect(view.target_shields).toBeNull();
  });
  it('shows distant shared Helm contacts at the rim without extending local radar or permissions', () => {
    const state = raw([]);
    state.blackboardKinds.helm = 'Helm'; state.blackboardKinds.tactical = 'TacticalRadar';
    state.blackboards.helm = { tracks: [], radar_range: 80 };
    const contact = sample({ contact_id: 'distant', position: [180, 0] });
    state.blackboards.tactical = { tracks: [contact] };
    const view = buildHelmConsoleState(state, ['helm']);
    expect(view.range).toBe(80);
    expect(view.blips).toEqual([expect.objectContaining({ uuid: 'distant', radar_x: .96, radar_y: 0, edge: true })]);
    expect(contact.position).toEqual([180, 0]);
    expect(JSON.stringify(view)).not.toContain('SECRET');
    const restricted = consoleReadState(state, { id: 'helm-only', systems: ['helm'], feeds: [] });
    expect(buildHelmConsoleState(restricted, ['helm']).blips).toEqual([]);
    state.blackboards.tactical.tracks = [{ ...contact, predicted: true }];
    expect(buildHelmConsoleState(state, ['helm']).blips[0]).toMatchObject({ edge: true, predicted: true, selectable: false });
    expect(projectSensorTracks([contact], { range: 80 }).blips).toEqual([]);
  });
  it('combines only delivered Helm and Tactical records, retaining separate unresolved contact identities', () => {
    const state = raw(); state.blackboardKinds.helm = 'Helm';
    state.blackboardKinds.tactical = 'TacticalRadar';
    state.blackboards.helm = { tracks: [sample({ contact_id: 'helm-contact' })], radar_range: 100 };
    state.blackboards.tactical = { tracks: [sample({ contact_id: 'tactical-contact' })] };
    const view = buildHelmConsoleState(state, ['helm']);
    expect(view.blips.map(row => row.uuid)).toEqual(['helm-contact', 'tactical-contact']);
    expect(JSON.stringify(view)).not.toContain('SECRET');
    const read = consoleReadState(state, { id: 'helm-only', systems: ['helm'], feeds: [] });
    expect(buildHelmConsoleState(read, ['helm']).blips.map(row => row.uuid)).toEqual(['helm-contact']);
    state.blackboards.helm.tracks = []; state.blackboards.tactical.tracks = [];
    expect(buildHelmConsoleState(state, ['helm']).blips).toEqual([]);
  });
});

it('exposes only the delivered dated altitude while planar range and speed remain unchanged', () => {
  const data = raw([sample({ elevation: { height: 17, error: .25, velocity: -2 }, velocity: [3,4] })]);
  const view = buildSensorsConsoleState(data, ['focus','broad']);
  expect(view.target_elevation).toEqual({ height: 17, error: .25, velocity: -2, sample_tick: 10 });
  expect(view.target_track.elevation).toEqual(view.target_elevation);
  expect(view.target_range).toBeCloseTo(Math.hypot(10,20));
  expect(view.target_speed).toBe(5);
  const tactical = {...data, blackboardKinds:{...data.blackboardKinds,broad:'TacticalRadar'}};
  expect(buildWeaponsConsoleState(tactical,['broad']).target_elevation).toEqual(view.target_elevation);
  document.body.innerHTML = '<ph-sensor-panel></ph-sensor-panel>';
  const panel = document.querySelector('ph-sensor-panel'); panel.state = view;
  expect(panel.shadowRoot.getElementById('scan-data').textContent).toContain(t('sensing.track.elevation.value',{height:17,error:.25}));
  expect(panel.shadowRoot.getElementById('scan-data').textContent).toContain(t('sensing.track.elevation.sample',{tick:10}));
  for (const elevation of [undefined, {height:NaN,error:1}, {height:4,error:-1}, {height:4,error:1,velocity:Infinity}]) {
    const unknown = buildSensorsConsoleState(raw([sample({ elevation })]), ['focus','broad']);
    expect(unknown.target_elevation).toBeNull();
    expect(unknown.target_track.elevation).toBeNull();
    panel.state = unknown;
    expect(panel.shadowRoot.getElementById('scan-data').textContent).not.toContain(t('sensing.track.elevation.title'));
  }
});
