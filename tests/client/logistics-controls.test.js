/** @vitest-environment jsdom */
import { describe, it, expect, vi, afterEach } from 'vitest';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { LOGISTICS_ACTION_IDS } from '../../gui/stations/logistics-actions.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { buildLogisticsConsoleState } from '../../gui/console-state.js';
import '../../gui/components/ph-logistics-controls.js';
import '../../gui/components/ph-stock-target.js';
const main = { id: 'hold', resource: 'torpedoes', role: 'main', level: 4, ceiling: 6, incoming: 0, target: 4, reserve: 1, compartment: 'hold' };
const local = { id: 'local', resource: 'torpedoes', role: 'local', system: 'actual-magazine', level: 2, ceiling: 6, incoming: 2, target: 6, reserve: 0, compartment: 'weapons' };
const view = () => ({ system_id: 'actual-handler', enabled: true, logistics_auto: false, stores: [main, local],
  deliveries: [{ id: 7, source: 'hold', destination: 'local', quantity: 2, stage: 'crossing', compartment: 'hold', progress: 0.5, return_requested: false }] });
const registryOptions = current => { let count = 0; return { getState: () => current, actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'c' + (++count) }) }; };
const state = () => ({ system_ids: ['actual-handler', 'actual-magazine'], system_families: { 'actual-handler': 'logistics', 'actual-magazine': 'tactical' },
  systems: { 'actual-handler': view(), 'actual-magazine': { stock_control: { system_id: 'actual-magazine', store: 'local', target: 6, ceiling: 6, auto: false } } } });
afterEach(() => { delete window.activateSemanticAction; document.body.replaceChildren(); });
describe('permitted logistics controls', () => {
  it('routes correlated main policy and local goals to their different actual owners', () => {
    const current = state(), send = vi.fn(), registry = createEngineeringActionRegistry({ ...registryOptions(current), sendAction: send });
    const activate = (id, detail) => registry.activate(id, { context: 'engineering', correlation: 'c', detail });
    activate(LOGISTICS_ACTION_IDS.reserve, { store: 'hold', amount: 2 });
    activate(LOGISTICS_ACTION_IDS.target, { store: 'local', amount: 5 });
    expect(send.mock.calls[0][1]).toMatchObject({ control_system_id: 'actual-handler', store: 'hold', amount: 2, correlation: 'c1' });
    expect(send.mock.calls[1][1]).toMatchObject({ control_system_id: 'actual-magazine', store: 'local', amount: 5, correlation: 'c2' });
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls[0]).toEqual(['ControlSystemCorrelated', { correlation: 'c1', target: 'actual-handler', payload: { type: 'SetStockReserve', data: { store: 'hold', amount: 2 } } }]);
    expect(wire.mock.calls[1][1].target).toBe('actual-magazine');
  });
  it('refuses local ownership bypass, delegated control, invalid quantities and unknown jobs', () => {
    const current = state(), send = vi.fn(), registry = createEngineeringActionRegistry({ ...registryOptions(current), sendAction: send });
    const activate = (id, detail) => registry.activate(id, { context: 'engineering', detail });
    delete current.systems['actual-magazine'];
    activate(LOGISTICS_ACTION_IDS.target, { store: 'local', amount: 5 });
    activate(LOGISTICS_ACTION_IDS.reserve, { store: 'local', amount: 1 });
    activate(LOGISTICS_ACTION_IDS.dispatch, { source: 'hold', destination: 'local', amount: 0.5 });
    activate(LOGISTICS_ACTION_IDS.return, { job: 8 });
    current.systems['actual-handler'].logistics_auto = true;
    activate(LOGISTICS_ACTION_IDS.enabled, { enabled: false }); expect(send).not.toHaveBeenCalled();
  });
  it('dispatches only visible stores and returns an actual current delivery', () => {
    const current = state(), send = vi.fn(), registry = createEngineeringActionRegistry({ ...registryOptions(current), sendAction: send });
    current.systems['actual-handler'].max_batch = 2;
    registry.activate(LOGISTICS_ACTION_IDS.dispatch, { context: 'engineering', detail: { source: 'hold', destination: 'local', amount: 3 } });
    expect(send).not.toHaveBeenCalled();
    registry.activate(LOGISTICS_ACTION_IDS.dispatch, { context: 'engineering', detail: { source: 'hold', destination: 'local', amount: 2 } });
    registry.activate(LOGISTICS_ACTION_IDS.return, { context: 'engineering', detail: { job: 7 } });
    expect(send.mock.calls.map(row => row[0])).toEqual(['logistics_dispatch', 'logistics_return']);
    expect(send.mock.calls[1][1]).toMatchObject({ job: 7, control_system_id: 'actual-handler' });
  });
  it('renders separate stock/incoming/shortage and preserves focused policy input across updates', () => {
    const activate = vi.fn(); window.activateSemanticAction = activate;
    const control = document.createElement('ph-logistics-controls'); document.body.append(control); control.state = view();
    expect(control.shadowRoot.textContent).toContain('shortage 2');
    const input = control.shadowRoot.querySelector('input'); input.focus(); input.value = '3';
    control.state = { ...view(), stores: [{ ...main, level: 3 }, local] };
    expect(control.shadowRoot.querySelector('input').value).toBe('3');
    control.shadowRoot.querySelector('label button').click();
    expect(activate).toHaveBeenCalledWith(LOGISTICS_ACTION_IDS.target, expect.objectContaining({ detail: { store: 'hold', amount: 3 } }));
    control.state = { ...view(), logistics_auto: true, max_batch: 2, max_active: 1,
      deliveries: [{ ...view().deliveries[0], blocked: 'logistics.blocked.access' }] };
    expect(control.shadowRoot.textContent).toContain('Access is blocked');
    expect(control.shadowRoot.querySelector('[data-key="manual-amount"]').max).toBe('2');
    expect([...control.shadowRoot.querySelectorAll('input,select,button')].every(node => node.disabled)).toBe(true);
    expect(control.shadowRoot.querySelectorAll('input')).toHaveLength(3); // only two main policy controls plus manual quantity
  });
  it('keeps local goal separate from loading targets and hides it on unconfigured hulls', () => {
    window.activateSemanticAction = vi.fn(); const control = document.createElement('ph-stock-target'); document.body.append(control);
    control.state = { store: 'local', target: 6, ceiling: 6, incoming: 2, reserved: 1, held: 1, auto: false };
    control.input.value = '4'; control.button.click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(LOGISTICS_ACTION_IDS.target, expect.objectContaining({ detail: { store: 'local', amount: 4 } }));
    control.state = { ...control.state, auto: true }; expect(control.button.disabled).toBe(true);
    control.state = null; expect(control.hidden).toBe(true);
  });
  it('retains manual route, amount and focus while active deliveries keep publishing progress', () => {
    window.activateSemanticAction = vi.fn(); const control = document.createElement('ph-logistics-controls'); document.body.append(control);
    const extra = { ...main, id: 'another-hold' }, another = { ...local, id: 'another-local' };
    control.state = { ...view(), stores: [main, extra, local, another] };
    control.shadowRoot.querySelector('[data-key="manual-source"]').value = extra.id;
    control.shadowRoot.querySelector('[data-key="manual-destination"]').value = another.id;
    const amount = control.shadowRoot.querySelector('[data-key="manual-amount"]'); amount.value = '3'; amount.focus();
    control.state = { ...view(), stores: [main, extra, local, another], deliveries: [{ ...view().deliveries[0], progress: 0.7 }] };
    expect(control.shadowRoot.querySelector('[data-key="manual-source"]').value).toBe(extra.id);
    expect(control.shadowRoot.querySelector('[data-key="manual-destination"]').value).toBe(another.id);
    expect(control.shadowRoot.activeElement.dataset.key).toBe('manual-amount'); expect(control.shadowRoot.activeElement.value).toBe('3');
    [...control.shadowRoot.querySelectorAll('button')].find(node => node.textContent === 'Dispatch batch').click();
    expect(window.activateSemanticAction).toHaveBeenLastCalledWith(LOGISTICS_ACTION_IDS.dispatch, expect.objectContaining({ detail: { source: extra.id, destination: another.id, amount: 3 } }));
  });
  it('selects a typed logistics publication and keeps unrelated stocks absent', () => {
    const projected = buildLogisticsConsoleState({ blackboards: { 'actual-handler': { ...view(), system_id: undefined }, secret: { stores: [{ id: 'secret' }] } },
      blackboardKinds: { 'actual-handler': 'Logistics', secret: 'Logistics' } }, ['actual-handler']);
    expect(projected.system_id).toBe('actual-handler'); expect(projected.stores.map(row => row.id)).toEqual(['hold', 'local']);
  });
});
