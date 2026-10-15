/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createEngineeringActionRegistry } from '../../gui/stations/engineering-actions.js';
import { PRODUCTION_ACTION_IDS } from '../../gui/stations/production-actions.js';
import { ActionFeedbackLifecycle } from '../../gui/action-feedback.js';
import { buildProductionConsoleState } from '../../gui/console-state.js';
import { dispatchConsoleAction } from '../../gui/action-map.js';
import '../../gui/components/ph-production-controls.js';
const recipe = { id: 'torpedoes', inputs: [{ store: 'parts', amount: 1 }], outputs: [{ store: 'ammo', amount: 1 }], work_secs: 10, ceiling: 6 };
const view = () => ({ system_id: 'actual-fabricator', production_auto: false, enabled: true, target: 4, selected_recipe: recipe.id, recipes: [recipe], active: { id: 1, recipe: recipe.id, progress: 0.2 } });
afterEach(() => { delete window.activateSemanticAction; document.body.replaceChildren(); });
describe('finite production standing orders', () => {
  it('sends correlated exact-owner goal/recipe commands and rejects delegated or impossible targets', () => {
    const state = { system_ids: ['actual-fabricator'], system_families: { 'actual-fabricator': 'production' }, systems: { 'actual-fabricator': view() } };
    let sequence = 0; const send = vi.fn(), registry = createEngineeringActionRegistry({ getState: () => state, sendAction: send, actionFeedback: new ActionFeedbackLifecycle({ correlation: () => 'production-' + (++sequence) }) });
    registry.activate(PRODUCTION_ACTION_IDS.target, { context: 'engineering', detail: { amount: 6 } });
    registry.activate(PRODUCTION_ACTION_IDS.recipe, { context: 'engineering', detail: { recipe: null, target: 0 } });
    const wire = vi.fn(); for (const [action, fields] of send.mock.calls) dispatchConsoleAction({ action, ...fields }, wire, vi.fn());
    expect(wire.mock.calls[0][1]).toMatchObject({ target: 'actual-fabricator', payload: { type: 'SetProductionTarget', data: { amount: 6 } } });
    expect(wire.mock.calls[1][1]).toMatchObject({ target: 'actual-fabricator', payload: { type: 'SelectProductionRecipe', data: { recipe: null, target: 0 } } });
    registry.activate(PRODUCTION_ACTION_IDS.recipe, { context: 'engineering', detail: { recipe: 'missing', target: 0 } });
    registry.activate(PRODUCTION_ACTION_IDS.target, { context: 'engineering', detail: { amount: 7 } });
    state.systems['actual-fabricator'].production_auto = true;
    registry.activate(PRODUCTION_ACTION_IDS.enabled, { context: 'engineering', detail: { enabled: false } });
    expect(send).toHaveBeenCalledTimes(2);
  });
  it('retains input/selection DOM and drafts while actual batch progress updates', () => {
    window.activateSemanticAction = vi.fn(); const control = document.createElement('ph-production-controls'); document.body.append(control); control.state = view();
    const input = control.target, select = control.recipe; input.focus(); input.value = '5';
    control.state = { ...view(), active: { ...view().active, progress: 0.8 }, blocked: 'production.blocked.work' };
    expect(control.target).toBe(input); expect(control.recipe).toBe(select); expect(control.shadowRoot.activeElement).toBe(input); expect(input.value).toBe('5');
    control.applyTarget.click(); expect(window.activateSemanticAction).toHaveBeenCalledWith(PRODUCTION_ACTION_IDS.target, expect.objectContaining({ detail: { amount: 5 } }));
    select.value = ''; select.dispatchEvent(new Event('change')); control.state = { ...view(), active: null };
    expect(select.value).toBe(''); expect(control.applyTarget.disabled).toBe(true); control.applyRecipe.click();
    expect(window.activateSemanticAction).toHaveBeenLastCalledWith(PRODUCTION_ACTION_IDS.recipe, expect.objectContaining({ detail: { recipe: null, target: 0 } }));
    control.state = { ...view(), production_auto: true }; expect([...control.shadowRoot.querySelectorAll('input,select,button')].every(node => node.disabled)).toBe(true);
  });
  it('selects only the typed owned fabrication publication', () => {
    const state = { blackboards: { 'actual-fabricator': view(), foreign: { target: 99 } }, blackboardKinds: { 'actual-fabricator': 'Production', foreign: 'Production' }, controlSources: { 'actual-fabricator': 'Human' } };
    expect(buildProductionConsoleState(state, ['actual-fabricator'])).toMatchObject({ system_id: 'actual-fabricator', target: 4, production_controllable: true });
    state.controlSources['actual-fabricator'] = 'Offline';
    expect(buildProductionConsoleState(state, ['actual-fabricator']).production_controllable).toBe(false);
  });
});
