/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { PORT_ACTION_IDS } from '../../gui/stations/port-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { buildUmbilicalConsoleState } from '../../gui/console-state.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import '../../gui/components/ph-port-service-controls.js';
const view = () => ({ system_id: 'actual-transfer', running: false, selected_resource: 'machinery_parts', port_maintenance: false,
  port: { partner_uuid: 'provider', visit: 7, policy: 'manual_offer', eligible: true,
    offers: [{ id: 'parts', resource: 'machinery_parts', unit_amount: 5, quota_remaining: 20, available_lots: 20, costs: [{ resource: 'alloy', amount: 1 }] }],
    stores: [{ id: 'hold-machinery-parts', role: 'main', resource: 'machinery_parts', compartment: 'support', level: 10, held: 0, incoming: 0, target: 22, ceiling: 102, headroom: 92 }], order: null } });
afterEach(() => { delete window.activateSemanticAction; document.body.replaceChildren(); });
describe('visit-bound finite port offers', () => {
  it('sends exact owned correlated orders and rejects stale visits, incompatible stores, delegated or automatic purchases', () => {
    let sequence = 0; const state = { system_ids: ['actual-transfer'], system_families: { 'actual-transfer': 'umbilical' }, systems: { 'actual-transfer': view() } };
    const send = vi.fn(), registry = createEngineeringActionRegistry({ getState: () => state, sendAction: send,
      actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'port-' + (++sequence) }) });
    state.systems['actual-transfer'].port.stores.push({ ...view().port.stores[0], id: 'second-main-parts' });
    const order = { partner_uuid: 'provider', visit: 7, lines: [{ offer: 'parts', store: 'hold-machinery-parts', lots: 3 }] };
    const activate = detail => registry.activate(PORT_ACTION_IDS.start, { context: 'engineering', detail }); activate(order);
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls[0][1]).toMatchObject({ target: 'actual-transfer', payload: { type: 'StartPortOrder', data: order } });
    for (const bad of [{ ...order, visit: 6 }, { ...order, partner_uuid: 'old-provider' }, { ...order, lines: [{ ...order.lines[0], lots: 21 }] },
      { ...order, lines: [{ ...order.lines[0], store: 'foreign' }] }, { ...order, lines: [{ ...order.lines[0], lots: 19 }] },
      { ...order, lines: [order.lines[0], { ...order.lines[0], store: 'second-main-parts' }] }, { ...order, lines: [order.lines[0], order.lines[0]] }, { ...order, lines: [null] }, { ...order, lines: [{ ...order.lines[0], lots: '3' }] }]) activate(bad);
    state.systems['actual-transfer'].port.policy = 'free_allied_replenishment'; activate(order);
    state.systems['actual-transfer'].umbilical_controllable = false;
    registry.activate(PORT_ACTION_IDS.maintenance, { context: 'engineering', detail: { enabled: true } }); expect(send).toHaveBeenCalledTimes(1);
  });
  it('autofills whole demand lots, preserves manual edits/focus and never starts on edit or snapshot', () => {
    window.activateSemanticAction = vi.fn(); const control = document.createElement('ph-port-service-controls'); document.body.append(control); control.state = view();
    const row = control._rows.get('parts'), amount = row.amount, store = row.store; expect(amount.value).toBe('2');
    amount.focus(); amount.value = '3'; amount.dispatchEvent(new Event('input'));
    control.state = { ...view(), port: { ...view().port, stores: [{ ...view().port.stores[0], level: 15 }], offers: [{ ...view().port.offers[0], quota_remaining: 19 }] } };
    expect(control._rows.get('parts').amount).toBe(amount); expect(row.store).toBe(store); expect(control.shadowRoot.activeElement).toBe(amount); expect(amount.value).toBe('3');
    expect(window.activateSemanticAction).not.toHaveBeenCalled(); control.start.click();
    expect(window.activateSemanticAction).toHaveBeenLastCalledWith(PORT_ACTION_IDS.start, expect.objectContaining({ detail: { partner_uuid: 'provider', visit: 7, lines: [{ offer: 'parts', store: 'hold-machinery-parts', lots: 3 }] } }));
    window.activateSemanticAction.mockClear(); control.start.dispatchEvent(new Event('pointerdown'));
    control.state = { ...view(), port: { ...view().port, visit: 8 } }; expect(control._rows.get('parts').amount.value).toBe('2');
    expect(control._rows.get('parts').amount).not.toBe(amount);
    control.start.click(); expect(window.activateSemanticAction).not.toHaveBeenCalled();
  });
  it('bounds edited purchases by actual receiving space, displays settled progress and keeps free goals separate', () => {
    const control = document.createElement('ph-port-service-controls'); document.body.append(control); control.state = view(); const row = control._rows.get('parts');
    row.amount.value = '19'; row.amount.dispatchEvent(new Event('input')); expect(control.start.disabled).toBe(true);
    control.state = { ...view(), port: { ...view().port, order: { active: true, lines: [{ offer: 'parts', store: 'hold-machinery-parts', requested_lots: 3, delivered_lots: 1 }], blocked: 'ports.blocked.no_cost_stock' } } };
    expect(control.start.disabled).toBe(true); expect(control.stop.hidden).toBe(false); expect(control.progress.textContent).toContain('1/3'); expect(control.message.textContent).toContain('Protected');
    control.state = { ...view(), port: { ...view().port, policy: 'free_allied_replenishment' } }; expect(control.start.hidden).toBe(true); expect(control.offers.hidden).toBe(true);
    control.state = { ...view(), umbilical_controllable: false }; expect([...control.shadowRoot.querySelectorAll('button,input,select')].every(node => node.disabled)).toBe(true);
  });
  it('projects only the selected typed Umbilical and its current Human ownership', () => {
    const state = { blackboards: { transfer: view(), foreign: { port: { visit: 999 } } }, blackboardKinds: { transfer: 'Umbilical', foreign: 'Umbilical' }, controlSources: { transfer: 'Human' } };
    expect(buildUmbilicalConsoleState(state, ['transfer'])).toMatchObject({ system_id: 'transfer', port: { visit: 7 }, umbilical_controllable: true });
    state.controlSources.transfer = 'Offline'; expect(buildUmbilicalConsoleState(state, ['transfer']).umbilical_controllable).toBe(false);
  });
});
