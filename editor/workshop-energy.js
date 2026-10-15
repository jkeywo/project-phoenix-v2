import { guardTransitReferences } from './workshop-transit.js';
import { guardPortServiceReferences } from './workshop-ports.js';
import { guardSensingReferences } from './workshop-sensing.js';
import { guardCommsRadioReferences } from './workshop-comms-radio.js';
import { guardFittingReferences } from './workshop-fitting.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { energyAliases } from './workshop-energy-model.js';
import { requireCoolantStore } from './workshop-energy-model.js';
import { configuredPowerTree } from './workshop-energy-ai.js';
import { writeStateTrees } from './workshop-state-trees.js';
export * from './workshop-energy-model.js';
const clone = value => structuredClone(value);

export function inspectEnergy(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  const profile = inspection.resolved.value.energy;
  return { ...inspection, energy: profile ? { ...clone(profile), consumer: clone(profile.consumer || []), group: clone(profile.group || []) } : null,
    systems: clone(authoredAndGeneratedSystems(inspection.resolved.value).filter(system => !energyAliases(system.kind))),
    groups: Object.keys(inspection.resolved.value.power_groups || {}).sort(),
    services: clone(inspection.resolved.value.resource_services || {}),
    capacities: clone(inspection.resolved.value.infrastructure?.capacity || []) };
}
export function replaceEnergySource(source, value) {
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
  let after = replaceEnergySource(before, energy);
  if (!inspection.energy) {
    requireCoolantStore(inspection.resolved.value, energy.reactor.coolant);
    for (const system of inspection.resolved.value.system || []) if (system.kind === 'power_reactor' && !system.config?.ai?.tree?.length)
      after = writeStateTrees(after, system, configuredPowerTree(energy), { materialise: operation.materialise === true });
  }
  guardPortServiceReferences(inspection.resolved.value, { ...inspection.resolved.value, energy });
  guardFittingReferences(inspection.resolved.value, { ...inspection.resolved.value, energy });
  guardSensingReferences(inspection.resolved.value, { ...inspection.resolved.value, energy });
  guardCommsRadioReferences(inspection.resolved.value, { ...inspection.resolved.value, energy });
  guardTransitReferences(inspection.resolved.value, { ...inspection.resolved.value, energy });
  parse(after);
  return { changes: [{ path: operation.path, before, after }] };
}
export async function applyEnergyOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-energy-operation',
    prepare: (captured, effective) => prepareEnergyOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectEnergy(candidate, effective, selected.path) });
}
