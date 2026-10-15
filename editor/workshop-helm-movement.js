import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
const object = value => value && typeof value === 'object' && !Array.isArray(value);
const roles = { thrust: 'helm_thrust', steering: 'helm_steering', lateral: 'lateral_thrust' };
export function validateHelmMovement(value, entity) {
  if (value === false) return;
  if (!object(value) || Object.keys(value).some(key => !(key in roles))
    || typeof value.thrust !== 'string' || typeof value.steering !== 'string'
    || (value.lateral !== undefined && typeof value.lateral !== 'string')) throw new Error('Choose closed actual thrust, steering and optional lateral System bindings, or false.');
  const systems = authoredAndGeneratedSystems(entity), ids = Object.values(value);
  if (new Set(ids).size !== ids.length || Object.entries(value).some(([role, id]) => !systems.some(system =>
    system.id === id && system.kind === roles[role] && entity.console?.some(console => console.id === system.console))))
    throw new Error('Movement bindings require distinct actual axis Systems with owning Consoles.');
}
export function inspectHelmMovement(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path), entity = inspection.resolved.value;
  const value = entity.helm_control ?? false; validateHelmMovement(value, entity);
  return { ...inspection, value, systems: authoredAndGeneratedSystems(entity).filter(row => Object.values(roles).includes(row.kind)),
    inherited: inspection.fields.some(row => (row.path === 'helm_control' || row.path.startsWith('helm_control.')) && row.owner !== path) };
}
export function guardHelmMovementReferences(_before, after) {
  if (after.helm_control !== undefined) validateHelmMovement(after.helm_control, after);
}
const ordered = value => Array.isArray(value) ? value.map(ordered) : object(value)
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])])) : value;
function write(source, value) {
  const before = parse(source), nl = source.includes('\r\n') ? '\r\n' : '\n';
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, text: match[0], name: match[1].replace(/\]$/, '').trim() }));
  const root = headers.filter(row => row.name === 'helm_control' || row.name.startsWith('helm_control.'));
  let after = source;
  for (const row of root.slice().reverse()) {
    const end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const comments = source.slice(row.start, end).split(/\r?\n/).map(line => line.match(/#[^\r\n]*/)?.[0]).filter(Boolean).join(nl);
    after = after.slice(0, row.start) + (comments ? comments + nl : '') + after.slice(end);
  }
  if (!root.length && before.helm_control !== undefined) {
    const end = headers[0]?.start ?? source.length, prefix = source.slice(0, end);
    const field = /^[ \t]*helm_control[ \t]*=[ \t]*(false|\{[^{}\r\n]*\})[^\r\n]*(?:\r?\n|$)/m.exec(prefix);
    if (!field) throw new Error('Expand movement bindings into ordinary TOML before editing.');
    const comment = field[0].match(/#[^\r\n]*/)?.[0];
    after = after.slice(0, field.index) + (comment ? comment + nl : '') + after.slice(field.index + field[0].length);
  }
  const text = value === false ? 'helm_control = false' + nl : stringify({ helm_control: value }).trim().replace(/\n/g, nl) + nl;
  // Root scalar must precede every table. Tables can be appended safely.
  after = value === false ? text + after : after + (after.endsWith(nl) ? nl : nl + nl) + text;
  const parsed = parse(after), without = row => { const copy = structuredClone(row); delete copy.helm_control; return JSON.stringify(ordered(copy)); };
  if (JSON.stringify(ordered(parsed.helm_control)) !== JSON.stringify(ordered(value)) || without(before) !== without(parsed))
    throw new Error('Expand movement bindings into ordinary TOML before editing.');
  return after;
}
export function prepareHelmMovementOperation(draft, dependencies, operation) {
  const inspection = inspectHelmMovement(draft, dependencies, operation.path);
  if (inspection.inherited && !operation.materialise) throw new Error('Explicitly materialise inherited movement bindings before editing.');
  validateHelmMovement(operation.value, inspection.resolved.value);
  const before = draft.read(operation.path), after = write(before, operation.value);
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
export async function applyHelmMovementOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => prepareHelmMovementOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectHelmMovement(candidate, effective, selected.path) });
}
