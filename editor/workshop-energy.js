import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

export const ENERGY_RESERVE_FIELDS = Object.freeze(['capacity', 'max_intake_per_sec', 'passive_loss_at_full_per_sec',
  'charge_heat_per_unit', 'discharge_heat_per_unit', 'passive_cooling_per_sec', 'coolant_heat_per_unit', 'trip_heat', 'recover_heat']);
export const ENERGY_REACTOR_FIELDS = Object.freeze(['sustainable_output_per_sec', 'max_output_per_sec', 'ramp_per_sec', 'initial_output_per_sec', 'initial_energy', 'initial_heat']);
export const ENERGY_CONSUMER_FIELDS = Object.freeze(['initial_energy', 'initial_heat', 'continuous_cost_per_sec', 'discrete_cost']);
export const ENERGY_GROUP_FIELDS = Object.freeze(['max_flow_per_sec', 'default_flow_per_sec', 'max_cooling_per_sec', 'default_cooling_per_sec', 'priority']);
const clone = value => structuredClone(value);
export const energyAliases = kind => ['power', 'power_reactor', 'power_battery'].includes(kind);
const reserve = () => ({ capacity: 100, max_intake_per_sec: 20, passive_loss_at_full_per_sec: 1, charge_heat_per_unit: 0.1,
  discharge_heat_per_unit: 0.1, passive_cooling_per_sec: 0.2, coolant_heat_per_unit: 10, trip_heat: 50, recover_heat: 30 });
export const defaultEnergyConsumer = system => ({ system, initial_energy: 50, initial_heat: 0, continuous_cost_per_sec: 1,
  discrete_cost: 1, reserve: reserve(), coolant: { service: 'coolant' } });
export const defaultEnergyGroup = id => ({ id, max_flow_per_sec: 20, default_flow_per_sec: 10,
  max_cooling_per_sec: 5, default_cooling_per_sec: 1, priority: 0 });
export function createEnergyModel(value) {
  return { reactor: { sustainable_output_per_sec: 100, max_output_per_sec: 200, ramp_per_sec: 20, initial_output_per_sec: 100,
    initial_energy: 50, initial_heat: 0, reserve: reserve(), coolant: { service: 'coolant' } },
  consumer: (value.system || []).filter(system => system.power_group && !energyAliases(system.kind)).map(system => defaultEnergyConsumer(system.id)),
  group: Object.keys(value.power_groups || {}).sort().map(defaultEnergyGroup) };
}
export function inspectEnergy(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  const profile = inspection.resolved.value.energy;
  return { ...inspection, energy: profile ? { ...clone(profile), consumer: clone(profile.consumer || []), group: clone(profile.group || []) } : null,
    systems: clone((inspection.resolved.value.system || []).filter(system => !energyAliases(system.kind))),
    groups: Object.keys(inspection.resolved.value.power_groups || {}).sort(),
    services: clone(inspection.resolved.value.resource_services || {}),
    capacities: clone(inspection.resolved.value.infrastructure?.capacity || []) };
}
function replaceEnergy(source, value) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'energy' || row.name.startsWith('energy.')).map(row => ({ start: row.start,
    end: headers.find(next => next.start > row.start)?.start ?? source.length }));
  if (!spans.length && parse(source).energy !== undefined) throw new Error('Expand inline energy definitions into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ energy: value }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function prepareEnergyOperation(draft, dependencies, operation) {
  const inspection = inspectEnergy(draft, dependencies, operation.path), before = draft.read(operation.path);
  if (inspection.fields.some(field => (field.path === 'energy' || field.path.startsWith('energy.')) && field.owner !== inspection.path)
    && !operation.materialise) throw new Error('Explicitly materialise inherited energy settings before editing.');
  const energy = clone(operation.energy);
  if (!energy?.reactor || !Array.isArray(energy.consumer) || !Array.isArray(energy.group)) throw new Error('Energy requires a reactor, consumers and control groups.');
  const after = replaceEnergy(before, energy); parse(after);
  return { changes: [{ path: operation.path, before, after }] };
}
export async function applyEnergyOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-energy-operation',
    prepare: (captured, effective) => prepareEnergyOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectEnergy(candidate, effective, selected.path) });
}
