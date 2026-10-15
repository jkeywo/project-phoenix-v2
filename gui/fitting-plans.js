/** Personal plans contain definition choices, never assemblies or simulation state. */
const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => keys.includes(key));
export function validatePlanner(context) {
  if (!context || typeof context.hull_path !== 'string' || !/^assets\/entities\/[^\\]+\.toml$/.test(context.hull_path) || context.hull_path.split('/').includes('..')
    || typeof context.content_id !== 'string' || !context.content_id || context.content_id.length > 256 || !Array.isArray(context.slots)) throw new Error('fitting.plan.invalid-context');
  const ids = new Set();
  for (const slot of context.slots) {
    if (!slot || ['slot', 'system', 'home_station', 'default_equipment'].some(key => typeof slot[key] !== 'string' || !slot[key]) || ids.has(slot.slot)
      || !Array.isArray(slot.compatible_equipment) || !slot.compatible_equipment.includes(slot.default_equipment)
      || slot.compatible_equipment.some(id => typeof id !== 'string' || !id) || new Set(slot.compatible_equipment).size !== slot.compatible_equipment.length) throw new Error('fitting.plan.invalid-context');
    ids.add(slot.slot);
  }
  return context;
}
export function defaultFittingPlan(context) {
  validatePlanner(context); const sections = Object.create(null);
  for (const slot of context.slots) (sections[slot.home_station] ||= { explicit: false, choices: Object.create(null) }).choices[slot.slot] = slot.default_equipment;
  return { version: 1, hull_path: context.hull_path, content_id: context.content_id, scope: '*', sections };
}
export function validateFittingPlan(plan, context) {
  validatePlanner(context);
  if (!closed(plan, ['version', 'hull_path', 'content_id', 'scope', 'sections']) || plan.version !== 1) throw new Error('fitting.plan.unsupported-version');
  if (plan.hull_path !== context.hull_path || plan.content_id !== context.content_id) throw new Error('fitting.plan.wrong-hull');
  const homes = [...new Set(context.slots.map(slot => slot.home_station))], expected = plan.scope === '*' ? homes : homes.includes(plan.scope) ? [plan.scope] : [];
  if (!expected.length || !closed(plan.sections, expected) || Object.keys(plan.sections).length !== expected.length) throw new Error('fitting.plan.invalid-choice');
  for (const station of expected) {
    const section = plan.sections[station], slots = context.slots.filter(slot => slot.home_station === station);
    if (!closed(section, ['explicit', 'choices']) || typeof section.explicit !== 'boolean' || !closed(section.choices, slots.map(slot => slot.slot))
      || Object.keys(section.choices).length !== slots.length || slots.some(slot => !slot.compatible_equipment.includes(section.choices[slot.slot]))) throw new Error('fitting.plan.invalid-choice');
  }
  return plan;
}
export function composeFittingPlans(current, imported, context) {
  validateFittingPlan(current, context); validateFittingPlan(imported, context);
  if (current.scope !== '*') throw new Error('fitting.plan.invalid-choice');
  const next = structuredClone(current);
  for (const [station, section] of Object.entries(imported.sections)) {
    if (imported.scope === '*' && next.sections[station].explicit) continue;
    next.sections[station] = { explicit: imported.scope !== '*' || section.explicit, choices: structuredClone(section.choices) };
  }
  return next;
}
export function stationFittingPlan(plan, context, station) {
  validateFittingPlan(plan, context);
  if (!plan.sections[station]) throw new Error('fitting.plan.invalid-choice');
  return { version: 1, hull_path: context.hull_path, content_id: context.content_id, scope: station,
    sections: { [station]: { explicit: true, choices: structuredClone(plan.sections[station].choices) } } };
}
export const fittingPlanStorageKey = context => { validatePlanner(context); return 'phoenix.fitting.plans.v1/' + JSON.stringify([context.hull_path, context.content_id]); };

/** Compare only received reports; the static catalog never reveals spare stocks. */
export function compareFittingPlan(plan, context, rows) {
  validateFittingPlan(plan, context);
  return context.slots.filter(slot => plan.sections[slot.home_station]).map(slot => {
    const reports = rows.flatMap(row => (row.fitting || []).map(view => ({ row, view }))).filter(record => record.view.slot === slot.slot && record.view.system === slot.system);
    const record = reports.find(record => record.row.system_id === slot.system) || reports[0];
    const choice = plan.sections[slot.home_station].choices[slot.slot], available = record?.view.choices?.find(item => item.equipment === choice)?.available;
    return { ...slot, choice, installed: record?.view.installed ?? null, available: available ?? null,
      requestable: !!record && !record.row.delegated && record.row.system_id === slot.system && record.view.can_request,
      owner: record?.row.system_id ?? null };
  });
}
