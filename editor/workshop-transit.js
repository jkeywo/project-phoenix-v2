import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { writeSystemComposition } from './workshop-system-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const closed = (value, keys) => object(value) && Object.keys(value).every(key => keys.includes(key));
const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value));
const positive = value => finite(value) && Math.fround(value) > 0;
const identity = value => typeof value === 'string' && /^[a-z0-9-]+$/.test(value);
const ordered = value => Array.isArray(value) ? value.map(ordered) : object(value)
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])])) : value;
const same = (a, b) => JSON.stringify(ordered(a)) === JSON.stringify(ordered(b));
export function validateTransitEntity(entity) {
  const systems = entity.system || [];
  for (const system of systems.filter(row => row.kind === 'transit-drive')) {
    const components = system.component || [], inputs = components.filter(row => row.kind === 'power_input');
    const drives = components.filter(row => row.kind === 'transit_drive'), drive = drives[0], config = drive?.config;
    if (!system.power_group || components.length !== 2 || inputs.length !== 1 || drives.length !== 1
      || (system.channel || []).length || !components.every(row => identity(row.id))
      || inputs[0].id === drive.id || drive.power_input !== inputs[0].id
      || !closed(inputs[0], ['kind', 'id']) || !closed(drive, ['kind', 'id', 'power_input', 'navigation_source', 'config'])
      || !closed(config, ['preparation_work_secs', 'max_distance', 'scatter_radius', 'gravity_response', 'world_travel', 'fuel', 'reading_ticks'])
      || !positive(config.preparation_work_secs) || !positive(config.max_distance)
      || !finite(config.scatter_radius) || config.scatter_radius < 0
      || (config.gravity_response !== undefined && (!finite(config.gravity_response) || config.gravity_response < 0))
      || (config.world_travel !== undefined && typeof config.world_travel !== 'boolean')
      || !closed(config.fuel, ['store', 'amount']) || typeof config.fuel.store !== 'string' || !config.fuel.store.trim()
      || !Number.isSafeInteger(config.fuel.amount) || config.fuel.amount <= 0
      || (config.reading_ticks !== undefined && (!Number.isInteger(config.reading_ticks) || config.reading_ticks < 0 || config.reading_ticks > 4294967295)))
      throw new Error('Transit needs one finite LocalJump drive and its actual power input, with no extra channels.');
    const navigation = systems.find(row => row.id === drive.navigation_source && row.kind === 'navigation');
    const console = entity.console?.find(row => row.id === system.console);
    if (!navigation || !console || (navigation.console !== console.id
      && !console.feed?.some(feed => feed.source === navigation.id && feed.kind === 'Navigation')))
      throw new Error('Choose an actual Navigation source owned by this Console or its explicit typed Navigation feed.');
    const logistics = systems.find(row => row.id === entity.logistics?.handling?.[0]?.system);
    const inventory = entity.console?.find(row => row.id === logistics?.console);
    if ([console, inventory].some(receiver => !receiver?.feed?.some(feed => feed.source === 'stock/' + system.id && feed.kind === 'Stock')))
      throw new Error('The drive and actual Logistics Console each require the explicit typed Stock feed for this local owner.');
    const consumer = entity.energy?.consumer?.find(row => row.system === system.id);
    if (!consumer || !positive(consumer.continuous_cost_per_sec) || !positive(consumer.discrete_cost))
      throw new Error('Transit needs positive finite continuous and discrete costs on its actual energy consumer.');
    const store = entity.logistics?.store?.find(row => row.id === config.fuel.store);
    const compartment = entity.interior?.placements?.[system.id];
    const capacity = entity.infrastructure?.capacity?.find(row => row.id === store?.endpoint?.id);
    if (!compartment || !store || store.role !== 'local' || store.system !== system.id || store.compartment !== compartment
      || store.endpoint?.kind !== 'capacity' || !capacity || !Number.isSafeInteger(capacity.ceiling) || capacity.ceiling < config.fuel.amount)
      throw new Error("Fuel requires this installed drive's same-room Local managed store and a sufficient actual Capacity endpoint.");
  }
  if (entity.arrival_gravity !== undefined && entity.arrival_gravity !== false) {
    const policy = entity.arrival_gravity;
    if (!closed(policy, ['exclusion_radius']) || !positive(policy.exclusion_radius))
      throw new Error('Arrival gravity must be false or a finite positive exclusion radius.');
    if (entity.tags?.includes('ship') || entity.collider?.movable || entity.helm_console || entity.behaviour || entity.civilian || entity.debris
      || systems.some(row => row.kind === 'transit-drive' || row.component?.some(component => component.kind === 'dock_mating' && component.config?.moves_body)))
      throw new Error('Arrival gravity supports fixed bodies only; moving equipment cannot supply it.');
  }
}
export function guardTransitReferences(_before, after) { validateTransitEntity(after); }
export function inspectTransit(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  validateTransitEntity(inspection.resolved.value);
  return { ...inspection, drives: (inspection.resolved.value.system || []).filter(row => row.kind === 'transit-drive') };
}
function candidate(draft, path, text) {
  return { paths: () => draft.paths(), isBinary: file => draft.isBinary(file), read: file => file === path ? text : draft.read(file) };
}
export function prepareTransitOperation(draft, dependencies, operation) {
  const inspection = inspectTransit(draft, dependencies, operation.path), before = draft.read(operation.path);
  if (operation.type === 'gravity') return prepareGravity(inspection, before, operation);
  const entity = inspection.resolved.value, system = inspection.drives.find(row => row.id === operation.owner);
  if (!system) throw new Error('Choose an actual installed Transit drive.');
  const prefix = `system[id=${system.id}]`;
  if (!operation.materialise && inspection.fields.some(row => (row.path === prefix || row.path.startsWith(prefix + '.')) && row.owner !== operation.path))
    throw new Error('Explicitly materialise the inherited drive before editing.');
  const components = structuredClone(system.component), drive = components.find(row => row.kind === 'transit_drive');
  drive.navigation_source = operation.navigation_source; drive.config = structuredClone(operation.config);
  const expected = structuredClone(entity), target = expected.system.find(row => row.id === system.id);
  target.component = components;
  if (system.channel !== undefined) target.channel = structuredClone(system.channel);
  validateTransitEntity(expected);
  let source = before;
  if (!(parse(source).system || []).some(row => row.id === system.id)) {
    if (!operation.materialise) throw new Error('Explicitly materialise the inherited drive before editing.');
    source += '\n' + stringify({ system: [{ id: system.id }] });
  }
  const after = writeSystemComposition(source, system.id, { components, channels: system.channel || [], materialise: operation.materialise }, system);
  const actual = inspectTransit(candidate(draft, operation.path, after), dependencies, operation.path).resolved.value;
  if (!same(expected, actual)) throw new Error('Expand this drive into ordinary TOML before editing; unrelated resolved data would change.');
  // Preserve existing comments even when the selected nested composition is serialized.
  const comments = [...before.matchAll(/#[^\r\n]*/g)].map(match => match[0]).filter(comment => !after.includes(comment));
  const preserved = after + (comments.length ? '\n' + comments.join('\n') + '\n' : '');
  if (!same(parse(after), parse(preserved))) throw new Error('Unable to preserve source comments safely.');
  return { changes: before === preserved ? [] : [{ path: operation.path, before, after: preserved }] };
}
export async function applyTransitOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => prepareTransitOperation(captured, effective, selected).changes,
    inspect: (captured, effective) => inspectTransit(captured, effective, selected.path) });
}

function prepareGravity(inspection, before, operation) {
  if (!operation.materialise && inspection.fields.some(row => (row.path === 'arrival_gravity' || row.path.startsWith('arrival_gravity.')) && row.owner !== operation.path))
    throw new Error('Explicitly materialise inherited arrival gravity before editing.');
  const expected = { ...inspection.resolved.value, arrival_gravity: structuredClone(operation.value) };
  validateTransitEntity(expected);
  const headers = [...before.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const gravity = headers.filter(row => row.name === 'arrival_gravity' || row.name.startsWith('arrival_gravity.'));
  let after = before;
  for (const row of gravity.slice().reverse()) {
    const end = headers.find(next => next.start > row.start)?.start ?? before.length;
    after = after.slice(0, row.start) + after.slice(end);
  }
  if (!gravity.length && parse(before).arrival_gravity !== undefined) {
    const end = headers[0]?.start ?? before.length;
    const field = /^[ \t]*arrival_gravity[ \t]*=[ \t]*(false|\{[^{}\r\n]*\})[^\r\n]*(?:\r?\n|$)/m.exec(before.slice(0, end));
    if (!field) throw new Error('Expand arrival gravity into ordinary TOML before editing.');
    after = after.slice(0, field.index) + after.slice(field.index + field[0].length);
  }
  const text = stringify({ arrival_gravity: operation.value });
  after = operation.value === false ? text + after : after + '\n' + text;
  const expectedLocal = { ...parse(before), arrival_gravity: operation.value };
  if (!same(parse(after), expectedLocal)) throw new Error('Expand arrival gravity into ordinary TOML before editing; unrelated source would change.');
  const comments = [...before.matchAll(/#[^\r\n]*/g)].map(match => match[0]).filter(comment => !after.includes(comment));
  after += comments.length ? '\n' + comments.join('\n') + '\n' : '';
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
