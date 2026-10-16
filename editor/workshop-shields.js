import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { defaultEnergyConsumer, defaultEnergyGroup, requireCoolantStore } from './workshop-energy-model.js';
import { replaceEnergySource } from './workshop-energy.js';
import { initialShieldTree } from './workshop-shield-ai.js';
import { shieldArcSystemId } from './workshop-generated-systems.js';
import { updateInteriorBindings } from './workshop-interiors.js';
import { defaultDamageComponent } from './workshop-damage-model.js';
import { replaceSectionSource } from './workshop-sensing.js';

export function validateShieldFrequency(value, entity) {
  const closed = (row, keys) => row && typeof row === 'object' && !Array.isArray(row) && Object.keys(row).every(key => keys.includes(key));
  const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value));
  if (!entity.shield_arc?.length || !closed(value,['response','cycle']) || !closed(value.response,['mismatch_loss'])
    || !finite(value.response.mismatch_loss) || value.response.mismatch_loss<0 || value.response.mismatch_loss>1) throw new Error('Shield frequency response requires actual arcs and finite mismatch loss in [0,1].');
  if (value.cycle !== false && (!closed(value.cycle,['frequencies','period_secs']) || !Array.isArray(value.cycle.frequencies)
    || value.cycle.frequencies.length<2 || value.cycle.frequencies.length>64 || !finite(value.cycle.period_secs) || Math.fround(value.cycle.period_secs)<=0 || Math.round(Math.fround(value.cycle.period_secs)*1e9)<=0 || Math.fround(value.cycle.period_secs)>=2**64
    || value.cycle.frequencies.some((row,i,all)=>!finite(row) || row<0 || row>1 || i>0 && Math.fround(row)===Math.fround(all[i-1])))) throw new Error('Shield cycle needs 2..64 frequencies in [0,1], a finite positive period, or explicit false.');
}
export function prepareShieldFrequencyOperation(draft, dependencies, operation) {
  const inspection = inspectShields(draft,dependencies,operation.path), before=draft.read(operation.path);
  if (!operation.materialise && inspection.fields.some(field=>(field.path==='shields_console' || field.path.startsWith('shields_console.')) && field.owner!==operation.path)) throw new Error('Explicitly materialise inherited shield policy before editing.');
  validateShieldFrequency(operation.value,inspection.resolved.value);
  const console={...structuredClone(inspection.resolved.value.shields_console || {}), frequency_response:structuredClone(operation.value.response),frequency_cycle:structuredClone(operation.value.cycle)};
  const after=replaceSectionSource(before,'shields_console',console);
  const own=parse(before), parsed=parse(after); delete own.shields_console; delete parsed.shields_console;
  if (JSON.stringify(own)!==JSON.stringify(parsed)) throw new Error('Expand shield policy into ordinary TOML tables before editing.');
  return {changes:[{path:operation.path,before,after}]};
}
export async function applyShieldFrequencyOperation({draft,provider,runtime,dependencies,operation,current=()=>true}) {
  const selected=structuredClone(operation);
  return acceptWorkshopChanges({draft,provider,runtime,dependencies,current,
    prepare:(captured,effective)=>prepareShieldFrequencyOperation(captured,effective,selected).changes,
    inspect:(candidate,effective)=>inspectShields(candidate,effective,selected.path)});
}

export const SHIELD_ENERGY_FIELDS = Object.freeze(['upkeep_per_sec', 'recharge_energy_per_hp', 'recovery_energy_per_hp', 'discharge_hp_per_sec']);
const clone = value => structuredClone(value);
const systemId = arc => shieldArcSystemId(arc.id);
export const defaultShieldArc = () => ({ id: '', label: '', focus_available: true, center_deg: 0, width_deg: 90, ai: initialShieldTree([]),
  damage_component: ['operation', 'intake', 'cooling'].map(binding => defaultDamageComponent(binding)),
  energy: { upkeep_per_sec: 0.05, recharge_energy_per_hp: 0.5, recovery_energy_per_hp: 0.25, discharge_hp_per_sec: 5, recovery_targets: [] } });
export function inspectShields(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, arcs: clone(inspection.resolved.value.shield_arc || []) };
}
function append(source, value) {
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl)
    + stringify(value).trim().replace(/\n/g, nl) + nl;
}
function spans(source) {
  return [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
}
function writeArc(source, arc, remove = false) {
  const headers = spans(source), records = headers.filter(row => row.name === 'shield_arc').map(row => ({ ...row,
    end: headers.find(next => next.start > row.start && !next.name.startsWith('shield_arc.'))?.start ?? source.length }));
  const record = records.find(row => parse(source.slice(row.start, row.end)).shield_arc?.[0]?.id === arc.id);
  if (!record && parse(source).shield_arc?.some(row => row.id === arc.id)) throw new Error('Expand inline shield arcs into TOML tables before editing.');
  if (!record) return remove ? source : append(source, { shield_arc: [arc] });
  const tail = source.slice(record.end), nl = source.includes('\r\n') ? '\r\n' : '\n';
  return source.slice(0, record.start) + (remove ? '' : stringify({ shield_arc: [arc] }).trim().replace(/\n/g, nl) + nl) + tail;
}
function removeUnusedLocalGroup(source, value, id) {
  const group = `energy-${id}`;
  const referenced = (value.system || []).some(system => system.power_group === group || JSON.stringify(system.config?.ai || {}).includes(group))
    || (value.energy.group || []).some(row => row.parent === group);
  if (referenced) return source;
  const headers = spans(source), header = headers.find(row => row.name === `power_groups.${group}` || row.name === `power_groups."${group}"`);
  // Included groups have no TOML-key tombstone. Keep their valid controls;
  // explicit materialisation of energy still removes the physical reserve.
  if (!header) return source;
  const end = headers.find(row => row.start > header.start)?.start ?? source.length;
  value.energy.group = value.energy.group.filter(row => row.id !== group);
  return source.slice(0, header.start) + source.slice(end);
}
function validate(arcs, value) {
  const byId = new Map(arcs.map(arc => [systemId(arc), arc]));
  if (byId.size !== arcs.length) throw new Error('Shield arc ids must be unique.');
  for (const arc of arcs) {
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(arc.id)) throw new Error('Choose a valid shield arc id.');
    if (!Number.isFinite(arc.center_deg) || !Number.isFinite(arc.width_deg) || arc.width_deg <= 0 || arc.width_deg > 360)
      throw new Error('Shield geometry must be finite, with width above zero and at most 360 degrees.');
    const work = arc.energy;
    if (!work || SHIELD_ENERGY_FIELDS.some(key => !Number.isFinite(work[key]) || work[key] < 0)
      || work.recharge_energy_per_hp <= 0) throw new Error('Shield work costs must be finite and non-negative, with positive recharge energy per HP.');
    if (work.recovery_energy_per_hp > 0 && work.recovery_energy_per_hp >= work.recharge_energy_per_hp)
      throw new Error('Shield recovery must return less energy than recharge consumes.');
    if (!Array.isArray(work.recovery_targets) || new Set(work.recovery_targets).size !== work.recovery_targets.length)
      throw new Error('Choose distinct shield recovery destinations.');
    for (const target of work.recovery_targets) {
      const recipient = byId.get(target);
      if (!recipient || !value.energy?.consumer?.some(row => row.system === target))
        throw new Error(`Recovery destination ${target} requires a shield arc and canonical local reserve.`);
      if (work.recovery_energy_per_hp > 0 && work.recovery_energy_per_hp >= recipient.energy?.recharge_energy_per_hp)
        throw new Error(`Recovery must return less energy than destination ${target} recharge consumes.`);
    }
  }
}
export function prepareShieldOperation(draft, dependencies, operation) {
  const inspection = inspectShields(draft, dependencies, operation.path), value = clone(inspection.resolved.value);
  const before = draft.read(operation.path), selected = inspection.arcs.find(arc => arc.id === operation.originalId);
  if (operation.originalId !== undefined && !selected) throw new Error('Selected shield arc no longer exists.');
  if (operation.remove && !selected) throw new Error('Select a shield arc to remove.');
  const arc = clone(operation.arc || selected);
  if (!arc?.id || (selected && arc.id !== selected.id)) throw new Error('Create another arc to use a different id.');
  if (!selected && inspection.arcs.some(row => row.id === arc.id)) throw new Error('Shield arc id already exists.');
  const prefix = `shield_arc[id=${arc.id}]`;
  const inherited = inspection.fields.some(field => (field.path === prefix || field.path.startsWith(prefix + '.')) && field.owner !== inspection.path);
  const changesHardware = !selected || operation.remove;
  const inheritedEnergy = inspection.fields.some(field => (field.path === 'energy' || field.path.startsWith('energy.')) && field.owner !== inspection.path);
  if ((inherited || (changesHardware && inheritedEnergy)) && !operation.materialise)
    throw new Error('Explicitly materialise inherited shield and energy settings before editing.');
  let after = before;
  if (operation.remove) {
    const id = systemId(selected);
    const references = ['console', 'station', 'system'].filter(key => JSON.stringify(value[key] || []).includes(JSON.stringify(id)));
    if (references.length) throw new Error(`Remove shield references from ${references.join(', ')} using Crew or Source before deleting the arc.`);
    value.shield_arc = inspection.arcs.filter(row => row.id !== arc.id);
    for (const row of value.shield_arc) if (row.energy?.recovery_targets?.includes(id)) {
      if (!operation.materialise && inspection.fields.some(field => field.path.startsWith(`shield_arc[id=${row.id}].`) && field.owner !== inspection.path))
        throw new Error('Explicitly materialise inherited recovery links before removing the arc.');
      row.energy.recovery_targets = row.energy.recovery_targets.filter(target => target !== id);
      if (operation.resetAi) row.ai = initialShieldTree(row.energy.recovery_targets);
      after = writeArc(after, row);
    }
    after = writeArc(after, selected, true);
    if (inherited) after = append(after, { shield_arc: [{ id: selected.id, _remove: true }] });
    value.energy = clone(value.energy);
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== id);
    after = removeUnusedLocalGroup(after, value, id);
    // Referenced/included groups store no reserve and remain valid controls.
    after = replaceEnergySource(after, value.energy);
  } else {
    if (!selected || operation.resetAi) arc.ai = initialShieldTree(arc.energy?.recovery_targets || []);
    value.shield_arc = selected ? inspection.arcs.map(row => row.id === selected.id ? arc : row) : [...inspection.arcs, arc];
    if (!selected) {
      if (!value.energy) throw new Error('Create the reactor and coolant hardware in Energy before adding shields.');
      requireCoolantStore(value);
      const id = systemId(arc), group = `energy-${id}`;
      if ((value.system || []).some(row => row.id === id) || value.energy.consumer?.some(row => row.system === id)
        || value.power_groups?.[group] || value.energy.group?.some(row => row.id === group))
        throw new Error('Generated shield System or allocation group already exists.');
      if (parse(before).power_groups && !spans(before).some(row => row.name.startsWith('power_groups.')))
        throw new Error('Expand inline power groups into TOML tables before adding shields.');
      const consumer = defaultEnergyConsumer(id); consumer.continuous_cost_per_sec = 0; consumer.discrete_cost = 0;
      value.energy.consumer = [...(value.energy.consumer || []), consumer];
      value.energy.group = [...(value.energy.group || []), defaultEnergyGroup(group)];
      after = replaceEnergySource(after, value.energy);
      after = append(after, { power_groups: { [group]: { label: arc.label || arc.id, default_level: 2, min_level: 0, max_level: 4 } } });
    }
    after = writeArc(after, arc);
  }
  validate(value.shield_arc, value);
  if (!selected || operation.remove || (operation.compartment !== undefined
    && inspection.resolved.value.interior?.placements?.[systemId(arc)] !== operation.compartment)) after = updateInteriorBindings(after, inspection, {
    ...(operation.remove ? { remove: [systemId(selected)] } : { add: { [systemId(arc)]: operation.compartment } }), materialise: operation.materialise === true });
  parse(after);
  return { changes: [{ path: operation.path, before, after }] };
}
export async function applyShieldOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-shield-operation',
    prepare: (captured, effective) => prepareShieldOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectShields(candidate, effective, selected.path) });
}
