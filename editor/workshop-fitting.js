import { validateWeaponTargets } from './workshop-weapon-model.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { validateEmpPayload } from './workshop-damage-model.js';

const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => keys.includes(key));
const named = value => typeof value === 'string' && /^[a-z0-9-]{1,128}$/.test(value);
const whole = (value, max = 4294967295) => Number.isSafeInteger(value) && value > 0 && value <= max;
const finite = value => Number.isFinite(Math.fround(value)) && value >= 0;
const positive = value => finite(value) && Math.fround(value) > 0;
export const healthyEquipment = () => ({ condition: { kind: 'healthy' }, dose: 0 });
export const emptyFitting = () => ({ slot: [], equipment: [], locker: [], assembly: [] });

export function fittingHardware(value, system) {
  if (system.kind === 'power_reactor') {
    const reactor = value.energy?.reactor;
    if (!reactor) throw new Error('Configure the actual reactor energy hardware first.');
    return { kind: 'reactor', ...Object.fromEntries(['sustainable_output_per_sec', 'max_output_per_sec', 'ramp_per_sec'].map(key => [key, reactor[key]])),
      charge_heat_per_unit: reactor.reserve.charge_heat_per_unit, discharge_heat_per_unit: reactor.reserve.discharge_heat_per_unit };
  }
  const generic = value.weapon?.find(weapon => weapon.delivery?.kind === 'beam' && 'phaser-' + (weapon.strategy_id || weapon.id) === system.id);
  const legacy = value.weapons_console?.phaser_banks?.find(bank => 'phaser-' + bank.id === system.id);
  const consumer = value.energy?.consumer?.find(row => row.system === system.id);
  if ((!generic && !legacy) || !consumer) throw new Error('Choose an actual beam bank with a canonical local reserve.');
  const bank = generic?.delivery || legacy;
  return { kind: 'beam', ...((generic || legacy).targets !== undefined && (generic || legacy).targets !== false ? { targets: structuredClone((generic || legacy).targets) } : {}), operation: bank.operation ?? 'fixed', beam_range: generic?.mount.range ?? bank.beam_range, auto_arc_deg: bank.auto_arc_deg,
    beam_damage_per_sec: bank.damage_per_sec ?? bank.beam_damage_per_sec, beam_duration_secs: bank.duration_secs ?? bank.beam_duration_secs,
    cooldown_secs: bank.cooldown_secs, cycle_jitter: bank.cycle_jitter ?? 0, shield_pierce: bank.shield_pierce ?? 0,
    impact: structuredClone(generic?.impact || bank.impact), continuous_cost_per_sec: consumer.continuous_cost_per_sec };
}
export function inspectFitting(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path), value = inspection.resolved.value;
  return { ...inspection, fitting: structuredClone(value.fitting || null), systems: authoredAndGeneratedSystems(value),
    rooms: value.interior?.compartment || [], services: value.resource_services || {} };
}
const compatible = (slot, equipment) => equipment.hardware.kind === slot.family && equipment.interface === slot.interface && equipment.size <= slot.size;
const sameHardware = (left, right) => { if (left.kind === 'beam') { left = { operation: 'fixed', ...left }; right = { operation: 'fixed', ...right }; if (left.targets === false) delete left.targets; if (right.targets === false) delete right.targets; }
  return Object.keys(left).length === Object.keys(right).length && Object.keys(left).every(key => typeof left[key] === 'number'
  ? Math.fround(left[key]) === Math.fround(right[key]) : key === 'targets' ? JSON.stringify(left.targets) === JSON.stringify(right.targets) : key === 'impact' ? left.impact?.damage_type === right.impact?.damage_type && Math.fround(left.impact?.penetration) === Math.fround(right.impact?.penetration)
    && !!left.impact?.emp === !!right.impact?.emp && (!left.impact?.emp || Math.fround(left.impact.emp.duration_secs) === Math.fround(right.impact.emp.duration_secs)) : left[key] === right[key]); };

export function validateFitting(profile, value) {
  if (!closed(profile, ['slot', 'equipment', 'locker', 'assembly'])) throw new Error('Fitting supports only slots, equipment, lockers and actual assemblies.');
  profile = { ...emptyFitting(), ...profile };
  for (const key of ['slot', 'equipment', 'locker', 'assembly']) if (!Array.isArray(profile[key])) throw new Error('Use explicit fitting lists.');
  if (!profile.slot.length && !profile.equipment.length && !profile.locker.length && !profile.assembly.length) return;
  if (!profile.slot.length || !profile.equipment.length || !value.interior || !value.energy || !value.damage) throw new Error('Fitting needs actual owned machinery, Energy, Damage and Interiors.');
  const systems = new Map(authoredAndGeneratedSystems(value).map(row => [row.id, row])), slots = new Map(), equipment = new Map(), lockers = new Map(), assemblies = new Set(), occupied = new Set(), space = new Map();
  const unique = (map, row) => { if (!named(row.id) || map.has(row.id)) throw new Error('Fitting identities must be distinct lowercase letters, digits and hyphens.'); map.set(row.id, row); };
  for (const slot of profile.slot) {
    if (!closed(slot, ['id', 'system', 'family', 'interface', 'size', 'default_equipment', 'operation_component']) || !named(slot.interface) || !whole(slot.size, 65535)) throw new Error('Slots need a closed family, interface, size and actual System.');
    unique(slots, slot);
    const system = systems.get(slot.system), kind = slot.family === 'beam' ? 'phaser_bank' : slot.family === 'reactor' ? 'power_reactor' : null;
    if (!system?.console || system.kind !== kind || profile.slot.some(other => other !== slot && other.system === slot.system)
      || !value.interior.placements?.[slot.system] || !system.damage_component?.some(component => component.id === slot.operation_component && component.binding === 'operation'))
      throw new Error('Bind each slot to one owned beam/reactor System, its room and explicit replaceable Operation component.');
    if (slot.family === 'reactor' && slot.system !== 'power-reactor') throw new Error('This reactor adapter requires the canonical power-reactor System.');
  }
  for (const definition of profile.equipment) {
    if (!closed(definition, ['id', 'interface', 'size', 'storage_space', 'work', 'hardware']) || !named(definition.interface) || !whole(definition.size, 65535) || !whole(definition.storage_space)) throw new Error('Equipment requires finite positive size and actual storage space.');
    unique(equipment, definition);
    const work = definition.work, hardware = definition.hardware;
    if (!closed(work, ['class', 'work_secs', 'manual_work_fraction', 'requirements']) || !['field', 'facility'].includes(work.class)
      || !positive(work.work_secs) || !positive(work.manual_work_fraction) || work.manual_work_fraction > 1 || !Array.isArray(work.requirements || [])) throw new Error('Choose finite field/facility work and a manual fraction in (0,1].');
    const requirements = new Set();
    for (const part of work.requirements || []) {
      const endpoint = value.resource_services?.[part.service] || { kind: 'capacity', id: part.service }, key = JSON.stringify([part.resource, part.service]);
      const managed = value.logistics?.store?.find(store => store.endpoint?.kind === endpoint.kind && store.endpoint?.id === endpoint.id);
      if (!closed(part, ['resource', 'service', 'amount']) || !named(part.resource?.replaceAll('_', '-')) || !named(part.service?.replaceAll('_', '-')) || !whole(part.amount, Number.MAX_SAFE_INTEGER)
        || requirements.has(key) || endpoint.kind !== 'capacity' || !value.infrastructure?.capacity?.some(capacity => capacity.id === endpoint.id)
        || (managed && managed.resource !== part.resource)) throw new Error('Use distinct positive part requirements backed by actual compatible finite services.');
      requirements.add(key);
    }
    if (hardware?.kind === 'beam') {
      if (!closed(hardware, ['kind', 'targets', 'operation', 'beam_range', 'auto_arc_deg', 'beam_damage_per_sec', 'beam_duration_secs', 'cooldown_secs', 'cycle_jitter', 'shield_pierce', 'impact', 'continuous_cost_per_sec'])
        || !['fixed','sustained'].includes(hardware.operation === undefined ? 'fixed' : hardware.operation) || hardware.operation === 'sustained' && hardware.cycle_jitter !== 0
        || !['beam_range', 'auto_arc_deg', 'beam_damage_per_sec', 'beam_duration_secs'].every(key => positive(hardware[key]))
        || !['cooldown_secs', 'continuous_cost_per_sec'].every(key => finite(hardware[key])) || !finite(hardware.cycle_jitter) || Math.fround(hardware.cycle_jitter) >= 1
        || !finite(hardware.shield_pierce) || hardware.shield_pierce > 1 || !closed(hardware.impact, ['damage_type', 'penetration', 'emp'])
        || !['physical', 'laser'].includes(hardware.impact.damage_type) || !finite(hardware.impact.penetration)) throw new Error('Beam hardware requires supported finite range, work, cycle and impact values.');
      validateWeaponTargets(hardware.targets);
      validateEmpPayload(hardware.impact.emp);
    } else if (hardware?.kind === 'reactor') {
      if (!closed(hardware, ['kind', 'sustainable_output_per_sec', 'max_output_per_sec', 'ramp_per_sec', 'charge_heat_per_unit', 'discharge_heat_per_unit'])
        || work.class !== 'facility' || !['sustainable_output_per_sec', 'max_output_per_sec'].every(key => finite(hardware[key])) || !positive(hardware.ramp_per_sec)
        || hardware.sustainable_output_per_sec > hardware.max_output_per_sec || value.energy.reactor.initial_output_per_sec > hardware.max_output_per_sec
        || !['charge_heat_per_unit', 'discharge_heat_per_unit'].every(key => finite(hardware[key]))) throw new Error('Reactor hardware needs facility work and compatible finite output, ramp and heat.');
    } else throw new Error('Only the supported beam and reactor fitting adapters can be authored.');
  }
  for (const slot of slots.values()) {
    if (!equipment.has(slot.default_equipment) || !compatible(slot, equipment.get(slot.default_equipment))) throw new Error('Each slot requires a compatible default definition.');
    const base = fittingHardware(value, systems.get(slot.system));
    if (!sameHardware(base, equipment.get(slot.default_equipment).hardware)) throw new Error('A slot default must reproduce its actual authored machinery. Create a sidegrade instead of silently changing the base hardware.');
    const mount = value.weapon?.find(row => row.delivery?.kind === 'beam' && 'phaser-' + (row.strategy_id || row.id) === slot.system)?.mount;
    const legacy = value.weapons_console?.phaser_banks?.find(row => 'phaser-' + row.id === slot.system);
    for (const definition of equipment.values()) if (compatible(slot, definition) && slot.family === 'beam' && definition.hardware.auto_arc_deg > (mount?.fire_arc_deg ?? legacy?.fire_arc_deg)) throw new Error('A sidegrade cannot exceed the fixed beam mount arc.');
  }
  for (const locker of profile.locker) {
    if (!closed(locker, ['id', 'compartment', 'space']) || !whole(locker.space) || !value.interior.compartment.some(room => room.id === locker.compartment)) throw new Error('Lockers need positive finite space in a real compartment.');
    unique(lockers, locker);
  }
  for (const item of profile.assembly) {
    if (!closed(item, ['id', 'definition', 'location', 'condition']) || !named(item.id) || item.id.length > 88 || assemblies.has(item.id) || !equipment.has(item.definition)) throw new Error('Each actual assembly needs a distinct bounded identity and known equipment definition.');
    assemblies.add(item.id); const definition = equipment.get(item.definition), location = item.location;
    if (location?.kind === 'installed') {
      const slot = slots.get(location.slot);
      if (!closed(location, ['kind', 'slot']) || !slot || occupied.has(slot.id) || !compatible(slot, definition) || item.definition !== slot.default_equipment || item.condition != null) throw new Error('Each slot must initially contain its real default assembly; installed conditions belong only to Damage.');
      occupied.add(slot.id);
    } else if (location?.kind === 'stored') {
      const locker = lockers.get(location.locker), condition = item.condition;
      if (!closed(location, ['kind', 'locker']) || !locker || !closed(condition, ['condition', 'dose', 'last_source']) || !Number.isFinite(condition.dose) || condition.dose < 0
        || (condition.last_source != null && (typeof condition.last_source !== 'string' || !condition.last_source.trim() || new TextEncoder().encode(condition.last_source).length > 256 || /[\x00-\x1f\x7f]/.test(condition.last_source)))
        || !['healthy', 'damaged', 'destroyed', 'patched'].includes(condition.condition?.kind)
        || !closed(condition.condition, condition.condition.kind === 'patched' ? ['kind', 'factor'] : ['kind'])
        || (condition.condition.kind === 'patched' && (!positive(condition.condition.factor) || condition.condition.factor > 1))) throw new Error('Stored equipment needs its supported real condition in a compatible finite locker.');
      const causal = condition.dose > 0 || condition.condition.kind !== 'healthy';
      if (causal && !condition.last_source) throw new Error('Damaged equipment or residual fault dose requires its actual damage source.');
      const supported = [...slots.values()].filter(slot => compatible(slot, definition)).some(slot => {
        const component = systems.get(slot.system).damage_component.find(component => component.id === slot.operation_component);
        const patch = value.damage?.remedies?.[component.patch_remedy]?.outcome;
        return condition.dose < Math.fround(component.fault_dose_threshold) && (condition.condition.kind !== 'patched'
          || patch?.kind === 'patch' && Math.fround(patch.factor) === Math.fround(condition.condition.factor));
      });
      if (!supported) throw new Error('Stored condition must fit a compatible Operation fault frontier.');
      space.set(locker.id, (space.get(locker.id) || 0) + definition.storage_space);
      if (!Number.isSafeInteger(space.get(locker.id)) || space.get(locker.id) > locker.space) throw new Error('Equipment locker capacity is exceeded.');
    } else throw new Error('Initial assemblies must be installed or stored, never invented in-progress work.');
  }
  if (occupied.size !== slots.size) throw new Error('Every authored slot requires its actual default installed assembly.');
}

export function replaceFittingSource(source, profile) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)].map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'fitting' || row.name.startsWith('fitting.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length; return { start: row.start, end };
  });
  if (!spans.length && parse(source).fitting !== undefined) throw new Error('Expand inline fitting into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ fitting: profile }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source; for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function guardFittingReferences(value, proposed) {
  if (!value.fitting) return;
  try { validateFitting(value.fitting, proposed); } catch (error) { throw new Error('Update the affected Fitting definition first: ' + error.message); }
}
export function prepareFittingOperation(draft, dependencies, operation) {
  const inspection = inspectFitting(draft, dependencies, operation.path), before = draft.read(operation.path);
  if (!operation.materialise && inspection.fields.some(row => (row.path === 'fitting' || row.path.startsWith('fitting.')) && row.owner !== operation.path)) throw new Error('Explicitly materialise inherited fitting before editing.');
  const profile = structuredClone(operation.value); validateFitting(profile, inspection.resolved.value);
  return { changes: [{ path: operation.path, before, after: replaceFittingSource(before, profile) }] };
}
export async function applyFittingOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-fitting-operation',
    prepare: (captured, effective) => prepareFittingOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectFitting(candidate, effective, selected.path) });
}
