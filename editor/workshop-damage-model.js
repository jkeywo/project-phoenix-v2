/** Closed authored damage vocabulary; the runtime remains the validator/owner. */
export const DAMAGE_TYPES = Object.freeze(['physical', 'laser']);
export const DAMAGE_BINDINGS = Object.freeze(['operation', 'intake', 'cooling']);
export const defaultEmpResistance = () => ({ resistance_per_stack: 0.5, max_stacks: 8, decay_secs: 20, bindings: ['operation'] });
export function validateEmpPayload(value) {
  if (value === undefined || value === null) return;
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).some(key => key !== 'duration_secs')
    || typeof value.duration_secs !== 'number' || !Number.isFinite(Math.fround(value.duration_secs)) || Math.fround(value.duration_secs) <= 0) throw new Error('EMP requires a finite positive duration.');
}
export function validateEmpResistance(value) {
  if (value === undefined || value === null) return;
  const positive = number => typeof number === 'number' && Number.isFinite(Math.fround(number)) && Math.fround(number) > 0;
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).length !== 4
    || Object.keys(value).some(key => !['resistance_per_stack', 'max_stacks', 'decay_secs', 'bindings'].includes(key))
    || !positive(value.resistance_per_stack) || !positive(value.decay_secs) || !Number.isInteger(value.max_stacks) || value.max_stacks < 1 || value.max_stacks > 65535
    || !Array.isArray(value.bindings) || !value.bindings.length || new Set(value.bindings).size !== value.bindings.length || value.bindings.some(binding => !DAMAGE_BINDINGS.includes(binding)))
    throw new Error('Choose finite positive EMP resistance/decay, bounded stacks and distinct actual component bindings.');
}
export const defaultImpact = kind => ({ damage_type: kind === 'beam' ? 'laser' : 'physical', penetration: kind === 'beam' ? 20 : kind === 'guided' ? 40 : 25 });
export const defaultDamage = () => ({ materials: {
  light: { resistance: { physical: 0.2, laser: 0.15 }, residual_fraction: 0.1 },
  bulkhead: { resistance: { physical: 1, laser: 0.7 }, residual_fraction: 0.1 },
  armour: { resistance: { physical: 2, laser: 1.4 }, residual_fraction: 0.1 },
}, remedies: {
  structural: { work_secs: 2, manual_work_fraction: 0.25, structural_hp: 10, outcome: { kind: 'structural' }, requirements: [{ resource: 'structural_parts', service: 'structural_parts', amount: 1 }] },
  'machinery-full': { work_secs: 4, manual_work_fraction: 0.25, outcome: { kind: 'full' }, requirements: [{ resource: 'machinery_parts', service: 'machinery_parts', amount: 1 }] },
  'machinery-patch': { work_secs: 3, manual_work_fraction: 0.5, outcome: { kind: 'patch', factor: 0.5 }, requirements: [{ resource: 'structural_parts', service: 'structural_parts', amount: 1 }] },
} });
export function defaultDamageComponent(binding = 'operation', id = binding) {
  return { id, binding, susceptibility: { physical: 1, laser: 1 }, fault_dose_threshold: 20,
    degraded_factor: 0.5, full_remedy: 'machinery-full', patch_remedy: 'machinery-patch', field_replaceable: true };
}
export function defaultSystemDamage(system, value = {}) {
  const operation = system.component?.find(row => ['dock_mating', 'access_boundary'].includes(row.kind))?.id || 'operation';
  const rows = [defaultDamageComponent('operation', operation)];
  if (system.kind === 'power_reactor' || value.energy?.consumer?.some(row => row.system === system.id)) {
    rows.push(defaultDamageComponent('intake', system.component?.find(row => row.kind === 'power_input')?.id || 'intake'), defaultDamageComponent('cooling'));
  }
  return rows;
}
const unit = value => Number.isFinite(value) && value >= 0 && value <= 1;
const positive = value => Number.isFinite(value) && value > 0;
const identity = value => typeof value === 'string' && /^[a-z0-9-]+$/.test(value);
export function validateDamage(damage, systems, value) {
  validateEmpResistance(damage?.emp);
  if (!damage?.materials || !damage?.remedies) throw new Error('Configure damage materials and repair remedies.');
  for (const [id, material] of Object.entries(damage.materials)) {
    if (!identity(id) || !unit(material.residual_fraction) || DAMAGE_TYPES.some(type => !Number.isFinite(material.resistance?.[type]) || material.resistance[type] < 0))
      throw new Error('Materials require physical/laser resistance and a residual fraction between zero and one.');
  }
  for (const [id, remedy] of Object.entries(damage.remedies)) {
    if (!identity(id) || !positive(remedy.work_secs) || !positive(remedy.manual_work_fraction) || remedy.manual_work_fraction > 1 || !['structural', 'full', 'patch'].includes(remedy.outcome?.kind)
      || (remedy.outcome.kind === 'structural' ? !positive(remedy.structural_hp) : remedy.structural_hp !== undefined)
      || (remedy.outcome.kind === 'patch' && (!positive(remedy.outcome.factor) || remedy.outcome.factor >= 1)) || !Array.isArray(remedy.requirements))
      throw new Error('Choose a supported bounded repair remedy and its physical work.');
    for (const requirement of remedy.requirements) {
      if (typeof requirement.resource !== 'string' || !requirement.resource.trim() || typeof requirement.service !== 'string' || !requirement.service.trim()
        || !Number.isSafeInteger(requirement.amount) || requirement.amount <= 0 || value.resource_services?.[requirement.service]?.kind !== 'capacity'
        || !value.infrastructure?.capacity?.some(stock => stock.id === value.resource_services[requirement.service].id))
        throw new Error('Repair parts require positive whole amounts and actual local capacity services.');
    }
  }
  for (const system of systems) {
    const ids = new Set(), bindings = new Set();
    if (!Array.isArray(system.damage_component) || !system.damage_component.length) throw new Error(`Author damage components for ${system.id}.`);
    for (const component of system.damage_component) {
      if (!identity(component.id) || ids.has(component.id) || !DAMAGE_BINDINGS.includes(component.binding)
        || !positive(component.fault_dose_threshold) || !positive(component.degraded_factor) || component.degraded_factor >= 1
        || DAMAGE_TYPES.some(type => !Number.isFinite(component.susceptibility?.[type]) || component.susceptibility[type] < 0)
        || typeof component.field_replaceable !== 'boolean' || damage.remedies[component.full_remedy]?.outcome.kind !== 'full'
        || (component.patch_remedy !== undefined && damage.remedies[component.patch_remedy]?.outcome.kind !== 'patch'))
        throw new Error(`Invalid or unsupported damage component on ${system.id}.`);
      if (component.binding !== 'operation' && system.kind !== 'power_reactor' && !value.energy?.consumer?.some(row => row.system === system.id))
        throw new Error(`Intake/cooling binding on ${system.id} requires an actual energy endpoint.`);
      ids.add(component.id); bindings.add(component.binding);
    }
    if (!bindings.has('operation')) throw new Error(`System ${system.id} needs an operation component.`);
  }
  for (const room of value.interior?.compartment || []) {
    if (!positive(room.max_hp) || !positive(room.recovery_fraction) || room.recovery_fraction >= 1 || !damage.materials[room.material]
      || damage.remedies[room.structural_remedy]?.outcome.kind !== 'structural') throw new Error(`Configure structure, material and structural remedy for ${room.id}.`);
  }
}
