/** @vitest-environment jsdom */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import '../../gui/components/ph-scan-readout.js';
import '../../gui/components/ph-passive-readout.js';
import { buildSensorsConsoleState } from '../../gui/console-state.js';
import { consoleReadState } from '../../gui/console-topology.js';
import { createClientSemanticActionRegistry } from '../../gui/client-semantic-actions.js';
import { createSensorScienceActionRegistry, SENSORS_SCAN_ACTION_ID, SENSORS_CANCEL_SCAN_ACTION_ID, SENSORS_PASSIVE_ACTION_ID } from '../../gui/stations/sensors-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { ACTION_MAP } from '../../gui/action-map.js';
import { t } from '../../gui/strings.js';
import fs from 'node:fs';
import { parse } from 'smol-toml';
import { resolveTemplate } from '../../editor/entity-includes.js';
import { renderStation as renderScience } from '../../gui/cruiser/science.console.js';

const reading = tick => ({ subject_uuid: 'known-contact', subject_name: 'world.entity.skyhook_depot.name', band: 'coarse', taken_at_tick: tick, condition_fraction: .5, condition_step: .25, mass: 10 });
const sample = { sample_id: 'opaque-sample', observed_tick: 20, bearing_rad: Math.PI / 2, bearing_error_rad: .1, strength: .4, signature: 'weapon' };
function view() { return { target_uuid: 'known-contact', scan: { system_id: 'focused-array', controllable: true, capable: true, reading: reading(20), history: [reading(10), reading(20)], focused: { subject: 'known-contact', progress: 250, waiting: 'unpowered' }, observation_tick: 25 }, passive: [{ system_id: 'receiver-x', enabled: true, can_operate: true, controllable: true, observation_tick: 25, signals: [sample] }] }; }
beforeEach(() => { document.body.innerHTML = ''; window.activateSemanticAction = vi.fn(); });
describe('scoped human sensing instruments', () => {
  it('keeps historical picker, focused control and disclosure nodes stable across progress snapshots', () => {
    document.body.innerHTML = '<ph-scan-readout></ph-scan-readout>'; const el = document.querySelector('ph-scan-readout'), state = view(); el.state = state;
    const picker = el.shadowRoot.getElementById('history'), cancel = el.shadowRoot.getElementById('cancel');
    picker.value = JSON.stringify(['known-contact', 10, 'coarse']); picker.dispatchEvent(new Event('change')); picker.focus();
    const details = el.shadowRoot.querySelector('details'); details.open = true;
    expect(el.shadowRoot.getElementById('progress').textContent).toContain('25%');
    el.state = { ...state, scan: { ...state.scan, history: [...state.scan.history, reading(30)], reading: reading(30), focused: { ...state.scan.focused, progress: 500 }, observation_tick: 30 } };
    expect(el.shadowRoot.activeElement).toBe(picker); expect(picker.value).toBe(JSON.stringify(['known-contact', 10, 'coarse'])); expect(el.shadowRoot.querySelector('details')).toBe(details);
    expect(details.open).toBe(true); expect(el.shadowRoot.getElementById('cancel')).toBe(cancel); expect(el.shadowRoot.getElementById('progress').textContent).toContain('50%');
    const key = vi.fn(); document.addEventListener('keydown', key);
    picker.dispatchEvent(new KeyboardEvent('keydown', { key: 'N', code: 'KeyN', shiftKey: true, bubbles: true, composed: true }));
    expect(key).not.toHaveBeenCalled(); document.removeEventListener('keydown', key);
    cancel.click(); expect(window.activateSemanticAction).toHaveBeenLastCalledWith(SENSORS_CANCEL_SCAN_ACTION_ID, { source: 'control', detail: { system_id: 'focused-array' } });
    el.state = { ...state, scan: { ...state.scan, controllable: false } }; expect(cancel.hidden).toBe(true); expect(el.shadowRoot.getElementById('action').hidden).toBe(true);
  });
  it('shows only measured passive bearing/signature/age and retains toggle DOM across samples', () => {
    document.body.innerHTML = '<ph-passive-readout></ph-passive-readout>'; const el = document.querySelector('ph-passive-readout'), state = view(); el.state = state;
    const button = el.shadowRoot.querySelector('button'); button.focus();
    expect(el.shadowRoot.textContent).toContain('90°'); expect(el.shadowRoot.textContent).toContain(t('passive.signature.weapon')); expect(el.shadowRoot.textContent).toContain(t('observation.ticks', { ticks: 5 })); expect(el.shadowRoot.textContent).not.toContain('opaque-sample');
    el.state = { passive: [{ ...state.passive[0], observation_tick: 28, signals: [sample, { ...sample, sample_id: 'other', observed_tick: 27 }] }] };
    expect(el.shadowRoot.querySelector('button')).toBe(button); expect(el.shadowRoot.activeElement).toBe(button);
    button.click(); expect(window.activateSemanticAction).toHaveBeenLastCalledWith(SENSORS_PASSIVE_ACTION_ID, { source: 'control', detail: { system_id: 'receiver-x', enabled: false } });
    el.state = { passive: [{ ...state.passive[0], controllable: false }] }; expect(button.hidden).toBe(true); expect(el.shadowRoot.querySelectorAll('li')).toHaveLength(1);
  });
  it('joins exact owner and permitted feed without turning feed access into control', () => {
    const raw = { blackboards: { scan: { owner: 'focused-array', capable: true, history: [reading(10)] }, 'receiver-x': { enabled: true, signals: [sample] }, hidden: { signals: [{ ...sample, sample_id: 'private' }] } }, blackboardKinds: { scan: 'Scan', 'receiver-x': 'Passive', hidden: 'Passive' }, controlSources: { 'focused-array': 'Human', 'receiver-x': 'Human' }, observationTick: 30 };
    const owned = ['focused-array', 'receiver-x']; let scoped = consoleReadState(raw, { id: 'science', systems: owned, feeds: [{ source: 'scan', kind: 'Scan' }] });
    let projected = buildSensorsConsoleState(scoped, owned); expect(projected.scan.controllable).toBe(true); expect(projected.scan.system_id).toBe('focused-array'); expect(projected.passive.map(row => row.system_id)).toEqual(['receiver-x']);
    projected = buildSensorsConsoleState(consoleReadState(raw, { id: 'visitor', systems: [], feeds: [{ source: 'scan', kind: 'Scan' }, { source: 'receiver-x', kind: 'Passive' }] }), []);
    expect(projected.scan.controllable).toBe(false); expect(projected.passive[0].controllable).toBe(false); expect(projected.scan.history).toHaveLength(1);
    delete raw.blackboards.scan.owner; scoped = consoleReadState(raw, { id: 'science', systems: owned, feeds: [{ source: 'scan', kind: 'Scan' }] }); expect(buildSensorsConsoleState(scoped, owned).scan.controllable).toBe(false);
  });
  it('correlates exact-owner actions, rejects stale and missing owners, and preserves authoritative refusal', () => {
    let state = view(), n = 0; const send = vi.fn(), feedback = new ActionFeedbackLifecycle({ correlation: () => 'sense-' + ++n });
    const registry = createSensorScienceActionRegistry({ getState: () => state, sendAction: send, actionFeedback: feedback });
    registry.activate(SENSORS_SCAN_ACTION_ID, { context: 'science', detail: { uuid: 'known-contact', system_id: 'focused-array' } });
    expect(send).toHaveBeenLastCalledWith('scan_target', expect.objectContaining({ control_system_id: 'focused-array', correlation: expect.any(String) }));
    const correlation = send.mock.calls[0][1].correlation; feedback.settle(correlation, 'Refused'); expect(feedback.get(correlation).state).toBe('Refused');
    send.mockClear(); registry.activate(SENSORS_CANCEL_SCAN_ACTION_ID, { context: 'science', detail: { system_id: 'old-owner' } }); registry.activate(SENSORS_SCAN_ACTION_ID, { context: 'science', detail: { system_id: 'old-owner' } }); registry.activate(SENSORS_PASSIVE_ACTION_ID, { context: 'science', detail: { system_id: 'hidden', enabled: false } }); expect(send).not.toHaveBeenCalled();
    state = { ...state, scan: { ...state.scan, system_id: null }, passive: [{ ...state.passive[0], controllable: false }] };
    registry.activate(SENSORS_SCAN_ACTION_ID, { context: 'science' }); registry.activate(SENSORS_PASSIVE_ACTION_ID, { context: 'science', detail: { system_id: 'receiver-x', enabled: false } }); expect(send).not.toHaveBeenCalled();
  });
  it('uses conflict-free Science/Sensors keyboard contexts and never sends an invented owner', () => {
    expect(() => createClientSemanticActionRegistry()).not.toThrow(); const send = vi.fn();
    ACTION_MAP.cancel_focused_scan({ control_system_id: 'focused-array', correlation: 'cancel' }, send); ACTION_MAP.set_passive_enabled({ control_system_id: 'receiver-x', enabled: false, correlation: 'passive' }, send);
    expect(send.mock.calls).toEqual([['ControlSystemCorrelated', { target: 'focused-array', correlation: 'cancel', payload: { type: 'CancelFocusedScan' } }], ['ControlSystemCorrelated', { target: 'receiver-x', correlation: 'passive', payload: { type: 'SetPassiveEnabled', data: { enabled: false } } }]]);
    send.mockClear(); ACTION_MAP.scan_target({ uuid: 'known-contact' }, send); ACTION_MAP.cancel_focused_scan({}, send); ACTION_MAP.set_passive_enabled({ enabled: true }, send); expect(send).not.toHaveBeenCalled();
  });
  it('mounts the actual Cruiser Science instruments and carries a normal correlated focused/cancel/passive journey', () => {
    const config = resolveTemplate('assets/entities/alliance_cruiser.toml', { read: path => fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : null }, parse).resolved.value;
    const definition = config.console.find(row => row.id === 'science'), ids = config.system.filter(row => row.console === definition.id).map(row => row.id);
    const focused = config.sensing.focused.system, receiver = config.sensing.passive.system;
    document.body.innerHTML = fs.readFileSync('gui/cruiser/science.html', 'utf8');
    const projected = buildSensorsConsoleState(consoleReadState({ observationTick: 40,
      blackboards: { scan: { owner: focused, capable: true, reading: reading(20), history: [reading(20)], focused: { subject: 'known-contact', progress: 500, waiting: '' } },
        [receiver]: { enabled: false, can_operate: true, signals: [sample] } }, blackboardKinds: { scan: 'Scan', [receiver]: 'Passive' },
      controlSources: Object.fromEntries(ids.map(id => [id, 'Human'])), sensorsTarget: 'known-contact' }, { ...definition, systems: ids, feeds: definition.feed }), ids);
    projected.target_uuid = 'known-contact'; let correlation = 0;
    const sent = vi.fn(), registry = createSensorScienceActionRegistry({ getState: () => projected, sendAction: sent,
      actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'cruiser-' + ++correlation }) });
    window.activateSemanticAction = (id, options) => registry.activate(id, { context: 'science', ...options });
    renderScience({ system_ids: ids, system_families: Object.fromEntries(ids.map(id => [id, 'sensors'])), systems: Object.fromEntries(ids.map(id => [id, projected])) }, document);
    document.querySelector('ph-scan-readout').shadowRoot.getElementById('action').click();
    document.querySelector('ph-scan-readout').shadowRoot.getElementById('cancel').click(); document.querySelector('ph-passive-readout').shadowRoot.querySelector('button').click();
    expect(sent.mock.calls.map(([name, data]) => [name, data.control_system_id])).toEqual([['scan_target', focused], ['cancel_focused_scan', focused], ['set_passive_enabled', receiver]]);
    for (const [, data] of sent.mock.calls) expect(data.correlation).toEqual(expect.any(String));
    expect(config.console.find(row => row.id === 'science').feed).toContainEqual({ source: receiver, kind: 'Passive' });
  });
});
