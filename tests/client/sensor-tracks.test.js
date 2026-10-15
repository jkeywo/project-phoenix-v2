/** @vitest-environment jsdom */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildSensorsConsoleState } from '../../gui/console-state.js';
import { consoleReadState } from '../../gui/console-topology.js';
import { projectSensorTracks } from '../../gui/sensor-tracks.js';
import { createSensorScienceActionRegistry, SENSORS_TARGET_ACTION_ID, SENSORS_SCAN_ACTION_ID } from '../../gui/stations/sensors-actions.js';
import '../../gui/components/ph-sensor-panel.js';
import { t } from '../../gui/strings.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';

const sample = (extra = {}) => ({ contact_id: 'contact-1', observed_tick: 10, position: [10, -20],
  position_error: 4, stage: 'return', predicted: false, ...extra });
function raw(tracks = [sample()]) {
  return { blackboards: { broad: { tracks, selected_target: 'contact-1' }, scan: { owner: 'focus', capable: true },
    sensors: { radar_range: 100 } }, blackboardKinds: { broad: 'SensorRadar', scan: 'Scan', sensors: 'Sensors' },
    observationTick: 30, sensorsTarget: 'secret-uuid', controlSources: { focus: 'Human' },
    asteroids: [{ uuid: 'secret-uuid', name: 'SECRET NAME', shipClass: 'SECRET CLASS', position: [2, 0, 3],
      radar_icon: 'battleship', radar_size: 999, faction: 'SECRET FACTION', shield_frequency: .9 }],
    regions: [{ uuid: 'secret-region', name: 'SECRET REGION' }], shipX: 0, shipZ: 0, shipYaw: 0 };
}
beforeEach(() => { document.body.innerHTML = ''; window.activateSemanticAction = vi.fn(); });
describe('permitted broad track radar', () => {
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
});
