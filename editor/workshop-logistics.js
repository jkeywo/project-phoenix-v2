import { guardTransitReferences } from './workshop-transit.js';
import { guardRecoveryReferences } from './workshop-resource-recovery.js';
import { guardProductionReferences } from './workshop-production.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { prepareShipOperation } from './workshop-ship-authoring.js';
import { prepareResourceOperation } from './workshop-resources.js';
import { requiredStockFeeds, stockPublicationSources, pruneStockFeedsSource } from './workshop-stock-bindings.js';

const clone = value => structuredClone(value);
const named = value => typeof value === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(value);
const quantity = value => Number.isSafeInteger(value) && value >= 0;
const closed = (value, keys, name) => {
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key)))
    throw new Error(`Unsupported ${name} fields.`);
};
export const emptyLogistics = () => ({ store: [], storage: [], handling: [] });
export const initialLogisticsTree = () => ({ root: 'logistics', tree: [{ id: 'logistics', initial: 'supply', state: [{ id: 'supply',
  task: [{ id: 'dispatch-needed', action: { kind: 'write', channel: 'dispatch', value: { source: 'fact', name: 'delivery_available' } } }] }] }] });
export function inspectLogistics(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path), value = inspection.resolved.value;
  return { ...inspection, logistics: clone(value.logistics || null), systems: clone(authoredAndGeneratedSystems(value)),
    compartments: clone(value.interior?.compartment || []), capacities: clone(value.infrastructure?.capacity || []),
    magazine: clone(value.torpedoes || null) };
}
export function replaceLogisticsSource(source, logistics) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'logistics' || row.name.startsWith('logistics.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length;
    return { start: row.start, end };
  });
  if (!spans.length && parse(source).logistics !== undefined) throw new Error('Expand inline logistics into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ logistics }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
/** Form feedback mirrors closed authored contracts. Runtime validation is final. */
export function validateLogistics(logistics, value) {
  closed(logistics, ['store', 'storage', 'handling'], 'logistics');
  if (!Array.isArray(logistics.store) || !Array.isArray(logistics.storage) || !Array.isArray(logistics.handling)
    || !logistics.store.length || logistics.handling.length !== 1) throw new Error('Logistics requires managed stores, storage and exactly one fitted handling System.');
  const systems = new Map(authoredAndGeneratedSystems(value).map(row => [row.id, row]));
  const rooms = new Set((value.interior?.compartment || []).map(row => row.id));
  const stores = new Map(), endpoints = new Set(), bays = new Map();
  for (const bay of logistics.storage) {
    closed(bay, ['compartment', 'space', 'unit_space'], 'storage');
    if (!rooms.has(bay.compartment) || bays.has(bay.compartment) || !quantity(bay.space) || bay.space <= 0)
      throw new Error('Storage requires one known compartment and positive whole shared space.');
    if (!bay.unit_space || Array.isArray(bay.unit_space) || !Object.keys(bay.unit_space).length
      || Object.entries(bay.unit_space).some(([id, cost]) => !id.trim() || !quantity(cost) || cost <= 0))
      throw new Error('Storage resource space costs must be named positive whole quantities.');
    bays.set(bay.compartment, { ...bay, occupied: 0 });
  }
  for (const store of logistics.store) {
    closed(store, ['id', 'endpoint', 'resource', 'compartment', 'role', 'system', 'source', 'target', 'reserve', 'priority'], 'managed store');
    if (!/^[a-z0-9-]+$/.test(store.id || '') || stores.has(store.id) || typeof store.resource !== 'string' || !store.resource.trim() || !rooms.has(store.compartment)
      || !quantity(store.target) || !quantity(store.reserve)) throw new Error('Stores require unique IDs, a resource, physical compartment and whole targets/reserves.');
    if (!Number.isSafeInteger(store.priority ?? 0) || (store.priority ?? 0) < 0 || (store.priority ?? 0) > 65535) throw new Error('Supply priority must fit an unsigned 16-bit value.');
    closed(store.endpoint, store.endpoint?.kind === 'capacity' ? ['kind', 'id'] : ['kind'], 'resource endpoint');
    const endpoint = store.endpoint, key = endpoint.kind === 'torpedo_magazine' ? 'torpedo_magazine' : `capacity:${endpoint.id}`;
    let amount, ceiling;
    if (endpoint.kind === 'torpedo_magazine' && store.resource === 'torpedoes' && value.torpedoes) {
      ceiling = value.torpedoes.count ?? 10; amount = value.torpedoes.initial_count ?? ceiling;
    } else if (endpoint.kind === 'capacity') {
      const capacity = (value.infrastructure?.capacity || []).find(row => row.id === endpoint.id);
      if (!capacity) throw new Error('Choose an actual capacity in Resources before binding this store.');
      amount = capacity.amount; ceiling = capacity.ceiling ?? amount;
    } else throw new Error('Choose an actual compatible capacity or torpedo magazine endpoint.');
    if (endpoints.has(key)) throw new Error('Each actual endpoint has exactly one managed location and resource.');
    endpoints.add(key);
    if (![amount, ceiling].every(quantity) || amount > ceiling || store.target > ceiling || store.reserve > ceiling)
      throw new Error('Store amounts, targets and protected reserves must fit the actual endpoint ceiling.');
    if (store.role === 'main') {
      if (store.system !== undefined || store.source !== undefined) throw new Error('Main stores belong to Inventory; remove local System/source fields.');
    } else if (store.role === 'local') {
      if (!systems.has(store.system) || !named(store.source) || value.interior?.placements?.[store.system] !== store.compartment)
        throw new Error('A local store requires its fitted owner in the same compartment and allowed main-store source.');
    } else throw new Error('Store role must be main or local.');
    const bay = bays.get(store.compartment), cost = bay && Object.hasOwn(bay.unit_space, store.resource) ? bay.unit_space[store.resource] : undefined;
    if (!cost) throw new Error('Declare compatible storage and resource space cost for every managed store.');
    bay.occupied += amount * cost;
    if (!Number.isSafeInteger(bay.occupied) || bay.occupied > bay.space) throw new Error('Actual initial stored quantities exceed shared compartment space.');
    stores.set(store.id, store);
  }
  for (const store of stores.values()) if (store.role === 'local') {
    const source = stores.get(store.source);
    if (!source || source.role !== 'main' || source.resource !== store.resource) throw new Error('Local stock sources must name a compatible main store.');
  }
  for (const handling of logistics.handling) {
    closed(handling, ['system', 'max_active', 'max_batch', 'units_per_sec', 'reorder_fraction'], 'handling');
    if (systems.get(handling.system)?.kind !== 'logistics') throw new Error('Fit a logistics System in Composition before assigning handling.');
    if (!Number.isSafeInteger(handling.max_active) || handling.max_active <= 0 || handling.max_active > 65535
      || !quantity(handling.max_batch) || handling.max_batch <= 0 || !Number.isFinite(Math.fround(handling.units_per_sec)) || Math.fround(handling.units_per_sec) <= 0
      || !Number.isFinite(Math.fround(Math.fround(handling.max_batch) / Math.fround(handling.units_per_sec))))
      throw new Error('Handling needs positive bounded slots, whole batch size and finite throughput.');
    if (!Number.isFinite(handling.reorder_fraction ?? 1) || (handling.reorder_fraction ?? 1) < 0 || (handling.reorder_fraction ?? 1) > 1)
      throw new Error('Reorder fraction must be finite and in [0,1].');
  }
}
export function prepareLogisticsOperation(draft, dependencies, operation, schema) {
  let inspection = inspectLogistics(draft, dependencies, operation.path);
  const priorStockSources = new Set(stockPublicationSources(inspection.resolved.value, schema));
  const before = draft.read(operation.path); let source = before;
  if (inspection.fields.some(field => (field.path === 'logistics' || field.path.startsWith('logistics.')) && field.owner !== inspection.path)
    && !operation.materialise) throw new Error('Explicitly materialise inherited logistics before editing.');
  let logistics = clone(operation.logistics);
  if (operation.type === 'logistics-create') {
    if (inspection.logistics) throw new Error('This entity already has logistics.');
    if (inspection.capacities.some(row => row.id === operation.capacity)) throw new Error('Choose a new main-store capacity ID; existing stocks are never overwritten by fitting.');
    if (!inspection.magazine) throw new Error('Create a real torpedo magazine in Resources before using this starter.');
    const local = inspection.systems.find(row => row.kind === 'torpedo_magazine');
    const localRoom = inspection.resolved.value.interior?.placements?.[local?.id];
    if (!local || !localRoom) throw new Error('Fit and place the actual magazine System before creating its supply route.');
    const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? source : draft.read(path) };
    source = prepareResourceOperation(candidate, dependencies, { path: operation.path, type: 'capacity-set', id: operation.capacity,
      amount: operation.amount, ceiling: operation.ceiling, materialise: operation.materialise }).changes[0].after;
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'system-add', id: operation.system,
      kind: 'logistics', console: operation.console, power_group: operation.group, compartment: operation.compartment,
      ai: initialLogisticsTree(), materialise: operation.materialise }).changes[0].after;
    const fittedValue = inspectLogistics(candidate, dependencies, operation.path).resolved.value;
    const fittedConsole = fittedValue.console.find(row => row.id === operation.console);
    const ownRatings = fittedConsole.rating?.length ? fittedConsole.rating : null;
    const ratings = ownRatings || fittedValue.station?.find(row => row.id === fittedConsole.home_station)?.rating || [];
    if (ratings.length)
      source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'rating-fit-system',
        ...(ownRatings ? { console: fittedConsole.id } : { station: fittedConsole.home_station }),
        system: operation.system }).changes[0].after;
    for (const feed of [{ source: operation.system, kind: 'Logistics' }, { source: local.id, kind: 'TorpedoMagazine' }]) {
      const console = inspectLogistics(candidate, dependencies, operation.path).resolved.value.console?.find(row => row.id === operation.console);
      if (!console?.feed?.some(row => row.source === feed.source && row.kind === feed.kind))
        source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'console-feed-add', id: operation.console, ...feed }).changes[0].after;
    }
    inspection = inspectLogistics(candidate, dependencies, operation.path);
    const count = inspection.magazine.count ?? 10;
    logistics = { store: [{ id: operation.capacity, endpoint: { kind: 'capacity', id: operation.capacity }, resource: 'torpedoes',
      compartment: operation.compartment, role: 'main', target: operation.amount, reserve: 0 },
    { id: 'magazine', endpoint: { kind: 'torpedo_magazine' }, resource: 'torpedoes', compartment: localRoom,
      role: 'local', system: local.id, source: operation.capacity, target: count, reserve: 0 }],
    storage: [{ compartment: operation.compartment, space: operation.ceiling, unit_space: { torpedoes: 1 } },
      ...(localRoom === operation.compartment ? [] : [{ compartment: localRoom, space: count, unit_space: { torpedoes: 1 } }])],
    handling: [{ system: operation.system, max_active: 1, max_batch: 1, units_per_sec: 1 }] };
    if (localRoom === operation.compartment) logistics.storage[0].space += count;
  }
  validateLogistics(logistics, inspection.resolved.value);
  source = replaceLogisticsSource(source, logistics);
  const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? source : draft.read(path) };
  const value = inspectLogistics(candidate, dependencies, operation.path).resolved.value;
  guardProductionReferences(inspection.resolved.value, value);
  guardRecoveryReferences(inspection.resolved.value, value);
  guardTransitReferences(inspection.resolved.value, value);
  const nextStockSources = new Set(stockPublicationSources(value, schema));
  source = pruneStockFeedsSource(source, value, new Set([...priorStockSources].filter(key => !nextStockSources.has(key))), operation.materialise);
  for (const feed of requiredStockFeeds(value, schema)) {
    const console = inspectLogistics(candidate, dependencies, operation.path).resolved.value.console?.find(row => row.id === feed.console);
    if (!console) throw new Error('Assign the local stock owner and Inventory to actual Consoles before editing stock bindings.');
    if (console.feed?.some(row => row.source === feed.source && row.kind === feed.kind)) continue;
    if (!parse(source).console?.some(row => row.id === feed.console)) {
      if (!operation.materialise) throw new Error('Explicitly materialise inherited Consoles before adding their required stock feeds.');
      const nl = source.includes('\r\n') ? '\r\n' : '\n'; source += nl + stringify({ console: [console] }).trim().replace(/\n/g, nl) + nl;
    }
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'console-feed-add', id: feed.console,
      source: feed.source, kind: feed.kind }).changes[0].after;
  }
  parse(source);
  return { changes: [{ path: operation.path, before, after: source }] };
}
export async function applyLogisticsOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-logistics-operation',
    prepare: async (captured, effective) => prepareLogisticsOperation(captured, effective, selected,
      typeof runtime.shipSchema === 'function' ? await runtime.shipSchema() : undefined).changes,
    inspect: (candidate, effective) => inspectLogistics(candidate, effective, selected.path) });
}
