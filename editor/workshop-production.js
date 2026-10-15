import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { prepareShipOperation } from './workshop-ship-authoring.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const closed = (row, keys) => row && typeof row === 'object' && !Array.isArray(row) && Object.keys(row).every(key => keys.includes(key));
const quantity = value => Number.isSafeInteger(value) && value >= 0;
export const initialProductionTree = () => ({ root: 'production', tree: [{ id: 'production', initial: 'manufacture', state: [{ id: 'manufacture', task: [
  { id: 'start-needed', when: [{ value: { source: 'fact', name: 'enabled' }, equals: true }],
    action: { kind: 'write', channel: 'start', value: { source: 'fact', name: 'production_available' } } },
] }] }] });
export function inspectProduction(draft, dependencies, path) {
  const result = inspectEntityComposition(draft, dependencies, path), value = result.resolved.value;
  return { ...result, production: structuredClone(value.production || null), systems: authoredAndGeneratedSystems(value),
    stores: structuredClone(value.logistics?.store || []), compartments: value.interior?.compartment || [] };
}
export function validateProduction(profile, value) {
  if (!closed(profile, ['system', 'initial_recipe', 'initial_target', 'recipe']) || !Array.isArray(profile.recipe) || !profile.recipe.length)
    throw new Error('Production requires a fitted System and at least one closed recipe.');
  if (authoredAndGeneratedSystems(value).find(row => row.id === profile.system)?.kind !== 'fabrication')
    throw new Error('Fit a fabrication System before assigning production.');
  const room = value.interior?.placements?.[profile.system];
  if (!room || !value.logistics) throw new Error('Place fabrication and create actual managed stores in Logistics first.');
  const stores = new Map(value.logistics.store.map(row => [row.id, row])), ids = new Set();
  const ceiling = store => store.endpoint.kind === 'torpedo_magazine' ? value.torpedoes?.count ?? 10
    : value.infrastructure?.capacity?.find(row => row.id === store.endpoint.id)?.ceiling
      ?? value.infrastructure?.capacity?.find(row => row.id === store.endpoint.id)?.amount;
  for (const recipe of profile.recipe) {
    if (!closed(recipe, ['id', 'inputs', 'outputs', 'work_secs']) || !/^[a-z0-9-]+$/.test(recipe.id || '') || ids.has(recipe.id)
      || !Number.isFinite(Math.fround(recipe.work_secs)) || Math.fround(recipe.work_secs) <= 0)
      throw new Error('Recipes need unique stable IDs and positive finite work time.');
    ids.add(recipe.id);
    for (const rows of [recipe.inputs, recipe.outputs]) {
      if (!Array.isArray(rows) || !rows.length) throw new Error('Recipes need actual material inputs and outputs.');
      const used = new Set();
      for (const amount of rows) {
        const store = stores.get(amount?.store);
        if (!closed(amount, ['store', 'amount']) || !store || store.compartment !== room || used.has(amount.store)
          || !quantity(amount.amount) || amount.amount === 0 || amount.amount > ceiling(store))
          throw new Error('Each recipe row needs a distinct managed store in the fabrication compartment and a quantity within its capacity.');
        used.add(amount.store);
      }
    }
  }
  if (!quantity(profile.initial_target) || (profile.initial_recipe == null ? profile.initial_target !== 0
    : !ids.has(profile.initial_recipe) || profile.initial_target > ceiling(stores.get(profile.recipe.find(row => row.id === profile.initial_recipe).outputs[0].store))))
    throw new Error('Initial production goal must fit the selected primary output store; no recipe requires target zero.');
}
export function replaceProductionSource(source, production) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'production' || row.name.startsWith('production.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length;
    return { start: row.start, end };
  });
  if (!spans.length && parse(source).production !== undefined) throw new Error('Expand inline production into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ production }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function guardProductionReferences(value, proposed) {
  if (!value.production) return;
  try { validateProduction(value.production, proposed); }
  catch (error) { throw new Error('Reassign the affected fabrication recipe, System or store in Production first: ' + error.message); }
}
export function prepareProductionOperation(draft, dependencies, operation, schema) {
  const before = draft.read(operation.path); let source = before;
  let inspection = inspectProduction(draft, dependencies, operation.path);
  if (!operation.materialise && inspection.fields.some(field => (field.path === 'production' || field.path.startsWith('production.')) && field.owner !== operation.path))
    throw new Error('Explicitly materialise inherited production before editing.');
  const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? source : draft.read(path) };
  const profile = structuredClone(operation.production);
  if (operation.type === 'production-create') {
    if (inspection.production) throw new Error('This hull already has production.');
    if (!inspection.resolved.value.logistics) throw new Error('Create real input/output stores in Logistics before fitting fabrication.');
    if (!inspection.resolved.value.energy?.reactor || !inspection.resolved.value.damage || !inspection.resolved.value.interior)
      throw new Error('Configure real energy, damage remedies and physical compartments before fitting fabrication.');
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'system-add', id: profile.system, kind: 'fabrication',
      console: operation.console, power_group: operation.group, compartment: operation.compartment, ai: initialProductionTree(), materialise: operation.materialise }).changes[0].after;
    inspection = inspectProduction(candidate, dependencies, operation.path);
    const console = inspection.resolved.value.console.find(row => row.id === operation.console);
    const ownRatings = console.rating?.length ? console.rating : null;
    const ratings = ownRatings || inspection.resolved.value.station?.find(row => row.id === console.home_station)?.rating || [];
    if (ratings.length) source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'rating-fit-system', system: profile.system,
      ...(ownRatings ? { console: console.id } : { station: console.home_station }) }).changes[0].after;
  }
  validateProduction(profile, inspectProduction(candidate, dependencies, operation.path).resolved.value);
  source = replaceProductionSource(source, profile);
  const value = inspectProduction(candidate, dependencies, operation.path).resolved.value;
  const system = authoredAndGeneratedSystems(value).find(row => row.id === profile.system), inventory = value.logistics.handling[0].system;
  const console = value.console.find(row => row.id === system.console);
  if (!console) throw new Error('Assign fabrication to an actual Console in Composition before editing production.');
  const required = [{ source: profile.system, kind: 'Production' }, { source: inventory, kind: 'Logistics' }];
  if (required.some(feed => !console.feed?.some(row => row.source === feed.source && row.kind === feed.kind)) && !parse(source).console?.some(row => row.id === console.id)) {
    if (!operation.materialise) throw new Error('Explicitly materialise the inherited fabrication Console before adding required feeds.');
    const nl = source.includes('\r\n') ? '\r\n' : '\n'; source += nl + stringify({ console: [console] }).trim().replace(/\n/g, nl) + nl;
  }
  for (const feed of required) if (!inspectProduction(candidate, dependencies, operation.path).resolved.value.console.find(row => row.id === console.id).feed?.some(row => row.source === feed.source && row.kind === feed.kind))
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'console-feed-add', id: console.id, ...feed }).changes[0].after;
  parse(source); return { changes: [{ path: operation.path, before, after: source }] };
}
export async function applyProductionOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-production-operation',
    prepare: async (captured, effective) => prepareProductionOperation(captured, effective, selected, await runtime.shipSchema()).changes,
    inspect: (candidate, effective) => inspectProduction(candidate, effective, selected.path) });
}
