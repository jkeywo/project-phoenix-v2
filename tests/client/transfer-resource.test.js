/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderUmbilicalPanel } from '../../gui/stations/contextual-controls.js';
import { createEngineeringActionRegistry, UMBILICAL_RESOURCE_ACTION_ID } from '../../gui/stations/engineering-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { buildUmbilicalConsoleState } from '../../gui/console-state.js';
import { t } from '../../gui/strings.js';

function fixture() {
  let state = { systems: { 'service-port': { system_id: 'service-port', capacity: 'reserve_fuel', selected_resource: 'reserve_fuel',
    allowed_resources: ['reserve_fuel', 'coolant'], running: true, umbilical_auto: false, rate: 20, operator_level: 30, partner_level: 40 } },
    system_ids: ['service-port'], system_families: { 'service-port': 'umbilical' } };
  const wire = vi.fn(), actions = createEngineeringActionRegistry({ getState: () => state,
    sendAction: (action, detail) => dispatchConsoleAction({ action, ...detail }, wire),
    actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'resource-1' }) });
  window.activateSemanticAction = (id, options) => actions.activate(id, { context: 'engineering', ...options });
  document.body.innerHTML = '<div id="umbilical-panel"><button id="umbilical-btn"></button><span id="umbilical-status"></span></div>';
  const render = () => renderUmbilicalPanel(state, document, t); render();
  return { wire, actions, render, get state() { return state; }, update(patch) { state = { ...state, systems: { 'service-port': { ...state.systems['service-port'], ...patch } } }; render(); } };
}
afterEach(() => { delete window.activateSemanticAction; document.body.innerHTML = ''; });
describe('actual-stock transfer resource selection', () => {
  it('routes a rendered coolant selection through the exact owner without mutating stocks or running intent', () => {
    const f = fixture(), select = document.querySelector('[data-transfer-resource]');
    expect([...select.options].map(option => option.value)).toEqual(['reserve_fuel', 'coolant']);
    expect(select.options[0].textContent).toBe(t('console.umbilical.reserve_fuel'));
    expect(select.options[1].textContent).toBe(t('energy.coolant'));
    select.value = 'coolant'; select.dispatchEvent(new Event('change'));
    expect(f.wire).toHaveBeenCalledWith('ControlSystemCorrelated', { correlation: 'resource-1', target: 'service-port',
      payload: { type: 'SetTransferResource', data: { resource: 'coolant' } } });
    expect(f.state.systems['service-port']).toMatchObject({ selected_resource: 'reserve_fuel', running: true, operator_level: 30, partner_level: 40 });
    f.update({ selected_resource: 'coolant', operator_level: 12, partner_level: 14 });
    expect(select.value).toBe('coolant'); expect(document.getElementById('umbilical-status').textContent).toContain('12 → 14');
  });
  it('refuses unavailable or delegated choices and keeps control identity through snapshots', () => {
    const f = fixture(), select = document.querySelector('[data-transfer-resource]');
    expect(f.actions.activate(UMBILICAL_RESOURCE_ACTION_ID, { context: 'engineering', detail: { resource: 'ammunition' } })).toMatchObject({ handled: false });
    f.update({ umbilical_auto: true }); expect(select.disabled).toBe(true);
    expect(f.actions.activate(UMBILICAL_RESOURCE_ACTION_ID, { context: 'engineering', detail: { resource: 'coolant' } })).toMatchObject({ handled: false });
    expect(document.querySelector('[data-transfer-resource]')).toBe(select); expect(f.wire).not.toHaveBeenCalled();
    f.update({ allowed_resources: [] }); expect(select.parentElement.hidden).toBe(true);
    f.update({ allowed_resources: ['custom_resource'], selected_resource: 'custom_resource' });
    expect(select.options[0].textContent).toBe('custom_resource');
  });
  it('projects the selected resource and choices alongside real levels from the actual System', () => {
    const bb = { capacity: 'reserve_fuel', selected_resource: 'coolant', allowed_resources: ['reserve_fuel', 'coolant'], operator_level: 12, partner_level: 14, running: true };
    expect(buildUmbilicalConsoleState({ blackboards: { 'service-port': bb }, blackboardKinds: { 'service-port': 'Umbilical' } }, ['service-port']))
      .toMatchObject({ system_id: 'service-port', ...bb });
  });
});
