import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { prepareShipOperation } from './workshop-ship-authoring.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const closed = (row, keys) => row && typeof row === 'object' && !Array.isArray(row) && Object.keys(row).every(key => keys.includes(key));
const resourceName = name => typeof name === 'string' && name.trim().length > 0;
export const initialRecoveryTree = () => ({ root: 'recovery', tree: [{ id: 'recovery', initial: 'recover', state: [{ id: 'recover', task: [
  { id: 'recover-needed', action: { kind: 'write', channel: 'recover', value: { source: 'fact', name: 'recovery_available' } } },
] }] }] });
export function inspectRecovery(draft, dependencies, path) {
  const result = inspectEntityComposition(draft, dependencies, path), value = result.resolved.value;
  return { ...result, recovery: structuredClone(value.recovery || null), recoverable: structuredClone(value.recoverable || null),
    systems: authoredAndGeneratedSystems(value), compartments: value.interior?.compartment || [], capacities: value.infrastructure?.capacity || [] };
}
export function validateRecoveryTool(tool, value) {
  if (!closed(tool, ['system', 'range', 'units_per_sec', 'resources'])
    || ![tool.range, tool.units_per_sec].every(number => Number.isFinite(Math.fround(number)) && Math.fround(number) > 0)
    || !Array.isArray(tool.resources) || !tool.resources.length || tool.resources.some(name => !resourceName(name))
    || new Set(tool.resources).size !== tool.resources.length) throw new Error('Recovery needs positive finite range/rate and distinct resource names.');
  if (authoredAndGeneratedSystems(value).find(row => row.id === tool.system)?.kind !== 'resource-recovery')
    throw new Error('Choose an actual fitted resource-recovery System.');
  if (!value.logistics || !value.interior?.placements?.[tool.system]) throw new Error('Create managed stores and place the tool in a physical compartment first.');
}
export function validateRecoverableSource(source, value) {
  if (!closed(source, ['availability', 'resources']) || !['always', 'disabled'].includes(source.availability)
    || !source.resources || typeof source.resources !== 'object' || Array.isArray(source.resources) || !Object.keys(source.resources).length)
    throw new Error('A recoverable source needs an availability condition and explicit public resource manifest.');
  const endpoints = new Set();
  for (const [resource, endpoint] of Object.entries(source.resources)) {
    if (!resourceName(resource) || !closed(endpoint, endpoint?.kind === 'capacity' ? ['kind', 'id'] : ['kind'])) throw new Error('Use actual finite resource endpoints.');
    let key;
    if (endpoint.kind === 'capacity' && resourceName(endpoint.id) && value.infrastructure?.capacity?.some(row => row.id === endpoint.id)) key = 'capacity:' + endpoint.id;
    else if (endpoint.kind === 'torpedo_magazine' && resource === 'torpedoes' && value.torpedoes) key = 'torpedo_magazine';
    else throw new Error('Bind an actual capacity, or torpedoes to an installed magazine. Create its finite stock in Resources first.');
    if (endpoints.has(key)) throw new Error('Different recoverable resources cannot alias one physical stock.');
    endpoints.add(key);
  }
}
/** These authored roots replace as a whole, just like their runtime include rule. */
export function replaceRecoverySource(source, key, value) {
  if (!['recovery', 'recoverable'].includes(key)) throw new Error('Unsupported recovery section.');
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === key || row.name.startsWith(key + '.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length;
    return { start: row.start, end };
  });
  if (!spans.length && parse(source)[key] !== undefined) throw new Error('Expand inline recovery definitions into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ [key]: value }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function guardRecoveryReferences(value, proposed) {
  try {
    if (value.recovery) validateRecoveryTool(value.recovery, proposed);
    if (value.recoverable) validateRecoverableSource(value.recoverable, proposed);
  } catch (error) { throw new Error('Reassign affected recovery bindings in Recovery first: ' + error.message); }
}
export function prepareRecoveryOperation(draft, dependencies, operation, schema) {
  let inspection = inspectRecovery(draft, dependencies, operation.path);
  const before = draft.read(operation.path); let source = before;
  const section = operation.section || 'recovery';
  if (!['recovery', 'recoverable'].includes(section)) throw new Error('Unsupported recovery section.');
  if (!operation.materialise && inspection.fields.some(field => (field.path === section || field.path.startsWith(section + '.')) && field.owner !== operation.path))
    throw new Error('Explicitly materialise inherited recovery definitions before editing.');
  const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? source : draft.read(path) };
  const profile = structuredClone(operation.value);
  if (operation.type === 'recovery-create') {
    if (section !== 'recovery' || inspection.recovery) throw new Error('This entity already has a recovery tool.');
    if (!inspection.resolved.value.energy?.reactor || !inspection.resolved.value.damage || !inspection.resolved.value.logistics)
      throw new Error('Create actual managed storage, energy and component remedies before fitting recovery.');
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'system-add', id: profile.system, kind: 'resource-recovery',
      console: operation.console, power_group: operation.group, compartment: operation.compartment, ai: initialRecoveryTree(), materialise: operation.materialise }).changes[0].after;
    inspection = inspectRecovery(candidate, dependencies, operation.path);
    const console = inspection.resolved.value.console.find(row => row.id === operation.console);
    const ownRatings = console.rating?.length ? console.rating : null;
    const ratings = ownRatings || inspection.resolved.value.station?.find(row => row.id === console.home_station)?.rating || [];
    if (ratings.length) source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'rating-fit-system', system: profile.system,
      ...(ownRatings ? { console: console.id } : { station: console.home_station }) }).changes[0].after;
  }
  const value = inspectRecovery(candidate, dependencies, operation.path).resolved.value;
  if (section === 'recovery') validateRecoveryTool(profile, value); else validateRecoverableSource(profile, value);
  source = replaceRecoverySource(source, section, profile);
  if (section === 'recovery') {
    const system = authoredAndGeneratedSystems(value).find(row => row.id === profile.system), console = value.console?.find(row => row.id === system.console);
    if (!console) throw new Error('Assign recovery to an actual Console in Composition first.');
    const inventory = value.logistics.handling[0].system;
    const systems = authoredAndGeneratedSystems(value);
    // Typed fixed publication keys are independent of the producer's System ID.
    // The authoritative content validator checks that the explicit binding is valid.
    const existingRadar = console.feed?.some(feed => ['SensorRadar', 'TacticalRadar'].includes(feed.kind));
    const sensor = systems.find(row => row.kind === 'sensor_radar');
    if (!existingRadar && !sensor) throw new Error('Fit a Sensor Radar or explicitly bind a supported Radar feed in Composition before fitting recovery.');
    const required = [{ source: profile.system, kind: 'Recovery' }, { source: inventory, kind: 'Logistics' },
      ...(!existingRadar ? [{ source: 'sensor-radar', kind: 'SensorRadar' }] : [])];
    if (required.some(feed => !console.feed?.some(row => row.source === feed.source && row.kind === feed.kind)) && !parse(source).console?.some(row => row.id === console.id)) {
      if (!operation.materialise) throw new Error('Explicitly materialise the inherited recovery Console before adding required feeds.');
      const nl = source.includes('\r\n') ? '\r\n' : '\n'; source += nl + stringify({ console: [console] }).trim().replace(/\n/g, nl) + nl;
    }
    for (const feed of required) if (!inspectRecovery(candidate, dependencies, operation.path).resolved.value.console.find(row => row.id === console.id).feed?.some(row => row.source === feed.source && row.kind === feed.kind))
      source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'console-feed-add', id: console.id, ...feed }).changes[0].after;
  }
  parse(source); return { changes: [{ path: operation.path, before, after: source }] };
}
export async function applyRecoveryOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-recovery-operation',
    prepare: async (captured, effective) => prepareRecoveryOperation(captured, effective, selected, await runtime.shipSchema()).changes,
    inspect: (candidate, effective) => inspectRecovery(candidate, effective, selected.path) });
}
