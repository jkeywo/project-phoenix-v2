/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildConsoleView, buildLocalStockViews, buildWeaponsConsoleState } from '../../gui/console-state.js';
import { mountLocalStockControls } from '../../gui/components/local-stock-controls.js';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { LOGISTICS_ACTION_IDS, registerLogisticsActions } from '../../gui/stations/logistics-actions.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
const stock = { id: 'repair-parts', resource: 'machinery_parts', level: 8, available: 7, held: 1, incoming: 2, target: 12, ceiling: 12, compartment: 'engineering' };
const source = (owner = 'repair') => ({ system: owner, stores: [stock] });
afterEach(() => { document.body.replaceChildren(); delete window.activateSemanticAction; });
describe('Console local stock controls', () => {
  it('uses only exact permitted typed publications and keeps co-hosted ownership independent', () => {
    const state = { consoleDefinitions: [
      { id: 'repairs', home_station: 'engineering', systems: ['repair'], feeds: [{ source: 'stock/repair', kind: 'Stock' }] },
      { id: 'inventory', home_station: 'engineering', systems: ['inventory'], feeds: [{ source: 'stock/repair', kind: 'Stock' }] },
      { id: 'other', home_station: 'engineering', systems: ['sensors'] },
    ], systemConsoleFamilies: { repair: 'repair', inventory: 'logistics', sensors: 'sensors' },
    controlSources: { repair: 'Human', inventory: 'Human' },
    blackboards: { 'stock/repair': source(), 'stock/secret': source('secret'), bad: source() },
    blackboardKinds: { 'stock/repair': 'Stock', 'stock/secret': 'Stock', bad: 'Stock' } };
    expect(buildConsoleView('repairs', state).local_stock).toMatchObject([{ system_id: 'repair', controllable: true }]);
    expect(buildConsoleView('inventory', state).local_stock).toMatchObject([{ system_id: 'repair', controllable: false }]);
    expect(buildConsoleView('other', state).local_stock).toEqual([]);
    expect(buildLocalStockViews(state, ['repair'])).toHaveLength(2); // rejects the noncanonical duplicate key
    state.controlSources.repair = 'Ai';
    expect(buildConsoleView('repairs', state).local_stock[0]).toMatchObject({ controllable: false, auto: true });
  });
  it('routes a generic stock goal to its actual local owner, while priority remains Inventory-owned', () => {
    const current = { local_stock: [{ ...stock, store: stock.id, system_id: 'repair', controllable: true, auto: false }],
      system_ids: ['inventory'], system_families: { inventory: 'logistics' },
      systems: { inventory: { system_id: 'inventory', stores: [{ ...stock, role: 'local' }] } } };
    let sequence = 0;
    const send = vi.fn(), registry = createEngineeringActionRegistry({ getState: () => current, sendAction: send,
      actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'stock-' + (++sequence) }) });
    // The Console core also installs common stock actions on a mixed-family screen.
    expect(() => registerLogisticsActions(registry, { getState: () => current, sendAction: send })).not.toThrow();
    registry.activate(LOGISTICS_ACTION_IDS.target, { context: 'stock', detail: { system: 'repair', store: stock.id, amount: 10 } });
    registry.activate(LOGISTICS_ACTION_IDS.priority, { context: 'engineering', detail: { store: stock.id, priority: 2 } });
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls[0][1]).toMatchObject({ target: 'repair', payload: { type: 'SetStockTarget', data: { store: stock.id, amount: 10 } } });
    expect(wire.mock.calls[1][1]).toMatchObject({ target: 'inventory', payload: { type: 'SetStockPriority', data: { store: stock.id, priority: 2 } } });
    current.local_stock[0].controllable = false;
    registry.activate(LOGISTICS_ACTION_IDS.target, { context: 'stock', detail: { system: 'repair', store: stock.id, amount: 1 } });
    registry.activate(LOGISTICS_ACTION_IDS.priority, { context: 'engineering', detail: { store: stock.id, priority: 65536 } });
    expect(send).toHaveBeenCalledTimes(2);
  });
  it('keeps the active local-goal input and focus through stock/progress snapshots, then removes delegated controls', () => {
    const activate = vi.fn(); window.activateSemanticAction = activate;
    const controls = mountLocalStockControls(document), view = { ...stock, store: stock.id, system_id: 'repair', controllable: true, auto: false };
    controls.update([view]); const goal = document.querySelector('ph-stock-target');
    goal.input.focus(); goal.input.value = '9';
    controls.update([{ ...view, level: 9, incoming: 1 }]);
    expect(document.querySelector('ph-stock-target')).toBe(goal); expect(goal.shadowRoot.activeElement).toBe(goal.input); expect(goal.input.value).toBe('9');
    goal.button.click(); expect(activate).toHaveBeenCalledWith(LOGISTICS_ACTION_IDS.target, expect.objectContaining({ context: 'stock', detail: { system: 'repair', store: stock.id, amount: 9 } }));
    expect(goal.summary.hidden).toBe(true); // generic summary already includes actual available/held data
    controls.update([{ ...view, controllable: false, auto: true }]); expect(goal.hidden).toBe(true);
    controls.update([]); expect(document.querySelector('details').hidden).toBe(true); controls.dispose();
    expect(document.querySelector('details')).toBeNull();
  });
  it('supersedes the narrow magazine goal only when its exact generic feed exists', () => {
    const state = { blackboards: { magazine: { stock_store: 'ammo', stock_target: 6 }, 'stock/magazine': source('magazine') },
      blackboardKinds: { magazine: 'TorpedoMagazine', 'stock/magazine': 'Stock' } };
    expect(buildWeaponsConsoleState(state, ['magazine']).stock_control).toBeNull();
    delete state.blackboardKinds['stock/magazine'];
    expect(buildWeaponsConsoleState(state, ['magazine']).stock_control).toMatchObject({ store: 'ammo' });
  });
});
