/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { compareFittingPlan, composeFittingPlans, defaultFittingPlan, fittingPlanStorageKey, stationFittingPlan, validateFittingPlan } from '../../gui/fitting-plans.js';
import { mountFittingPlans } from '../../gui/components/fitting-plan-controls.js';
const catalog = () => ({ hull_path: 'assets/entities/cruiser.toml', content_id: 'resolved-hull-definition-hash', slots: [
  { slot: 'beam-slot', system: 'actual-beam', home_station: 'tactical', default_equipment: 'beam', compatible_equipment: ['beam', 'beam-efficient'] },
  { slot: 'reactor-slot', system: 'power-reactor', home_station: 'engineering', default_equipment: 'reactor', compatible_equipment: ['reactor', 'reactor-efficient'] },
] });
afterEach(() => { document.body.replaceChildren(); window.localStorage.clear(); delete window.activateSemanticAction; });
describe('personal fitting plans', () => {
  it('keys by the actual hull definition and preserves explicit Station sections on whole import', () => {
    const context = catalog(), defaults = defaultFittingPlan(context), mine = stationFittingPlan(defaults, context, 'tactical');
    mine.sections.tactical.choices['beam-slot'] = 'beam-efficient';
    const explicit = composeFittingPlans(defaults, mine, context), imported = defaultFittingPlan(context);
    imported.sections.engineering.choices['reactor-slot'] = 'reactor-efficient';
    const merged = composeFittingPlans(explicit, imported, context);
    expect(merged.sections.tactical).toEqual(mine.sections.tactical);
    const restored = composeFittingPlans(defaultFittingPlan(context), explicit, context);
    expect(composeFittingPlans(restored, imported, context).sections.tactical).toEqual(mine.sections.tactical); expect(merged.sections.engineering.choices['reactor-slot']).toBe('reactor-efficient');
    expect(fittingPlanStorageKey(context)).not.toBe(fittingPlanStorageKey({ ...context, content_id: 'another-definition' }));
    expect(fittingPlanStorageKey(context)).not.toBe(fittingPlanStorageKey({ ...context, hull_path: 'assets/entities/destroyer.toml' }));
  });
  it.each([
    plan => { plan.version = 2; }, plan => { plan.content_id = 'old'; },
    plan => { plan.sections.tactical.choices['beam-slot'] = 'free-item'; },
    plan => { plan.assemblies = [{ id: 'invented' }]; },
    plan => { plan.sections.tactical.choices.unknown = 'beam'; },
  ])('refuses unsupported plan data %# without mutating the current plan', mutate => {
    const context = catalog(), current = defaultFittingPlan(context), before = structuredClone(current), imported = structuredClone(current); mutate(imported);
    expect(() => composeFittingPlans(current, imported, context)).toThrow(); expect(current).toEqual(before);
  });
  it('never infers unobserved stocks from the public catalog or a foreign report', () => {
    const context = catalog(), plan = defaultFittingPlan(context);
    expect(compareFittingPlan(plan, context, [])[0]).toMatchObject({ available: null, installed: null, requestable: false });
    const reports = [{ system_id: 'repair', fitting: [{ slot: 'beam-slot', system: 'actual-beam', installed: 'beam', can_request: false, choices: [{ equipment: 'beam', available: 2 }] }] },
      { system_id: 'actual-beam', fitting: [{ slot: 'beam-slot', system: 'actual-beam', installed: 'beam', can_request: true, choices: [{ equipment: 'beam', available: 2 }] }] }];
    expect(compareFittingPlan(plan, context, reports)[0]).toMatchObject({ available: 2, requestable: true, owner: 'actual-beam' });
  });
  it('preserves a claimant manual edit made in whole-ship scope during a later whole import', () => {
    const context = catalog(), widget = mountFittingPlans(document); document.body.append(widget.node);
    widget.update([{ system_id: 'actual-beam', fitting_planner: context }], 'tactical');
    const equipment = [...widget.node.querySelectorAll('select')].find(select => [...select.options].some(option => option.value === 'beam-efficient'));
    equipment.value = 'beam-efficient'; equipment.dispatchEvent(new Event('change'));
    const imported = defaultFittingPlan(context); imported.sections.engineering.choices['reactor-slot'] = 'reactor-efficient';
    widget.node.querySelector('textarea').value = JSON.stringify(imported);
    widget.node.querySelector('[data-plan-action=import]').click(); widget.node.querySelector('[data-plan-action=export]').click();
    const result = JSON.parse(widget.node.querySelector('textarea').value);
    expect(result.sections.tactical).toEqual({ explicit: true, choices: { 'beam-slot': 'beam-efficient' } });
    expect(result.sections.engineering.choices['reactor-slot']).toBe('reactor-efficient');
    widget.dispose();
  });
  it('persists and shares only plan data, then submits ordinary requests for the actual claimed home only', () => {
    const context = catalog(), widget = mountFittingPlans(document); document.body.append(widget.node);
    const rows = [{ system_id: 'actual-beam', fitting_planner: context, fitting: [{ slot: 'beam-slot', system: 'actual-beam', installed: 'beam', can_request: true, choices: [{ equipment: 'beam-efficient', available: 0 }] }] }];
    widget.update(rows, 'tactical'); const pickers = widget.node.querySelectorAll('select');
    pickers[0].value = 'tactical'; pickers[0].dispatchEvent(new Event('change'));
    const equipment = [...widget.node.querySelectorAll('select')].find(select => [...select.options].some(option => option.value === 'beam-efficient'));
    equipment.value = 'beam-efficient'; equipment.dispatchEvent(new Event('change'));
    widget.node.querySelector('input').value = 'My Tactical'; widget.node.querySelector('[data-plan-action=save]').click();
    const stored = JSON.parse(window.localStorage.getItem(fittingPlanStorageKey(context)));
    expect(stored.favourites[0].plan.scope).toBe('tactical'); expect(stored.favourites[0].plan).not.toHaveProperty('assemblies');
    widget.node.querySelector('[data-plan-action=export]').click(); const exported = JSON.parse(widget.node.querySelector('textarea').value);
    expect(() => validateFittingPlan(exported, context)).not.toThrow();
    window.activateSemanticAction = vi.fn(() => true); widget.node.querySelector('[data-plan-action=queue]').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith('fitting.request', expect.objectContaining({ context: 'fitting', detail: { owner: 'actual-beam', slot: 'beam-slot', equipment: 'beam-efficient' } }));
    expect(rows[0].fitting[0].installed).toBe('beam'); // Requests grant neither a spare nor installation.
    widget.update(rows, 'captain'); expect(widget.node.querySelector('[data-plan-action=queue]').disabled).toBe(true);
    widget.dispose();
  });
});
