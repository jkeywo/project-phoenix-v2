import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
const kinds = ['phaser_bank', 'blaster_bank', 'torpedo_tube'];
export function validateWeaponGroups(groups, entity) {
  if (!Array.isArray(groups)) throw new Error('Weapon groups must be a list.');
  const ids = new Set(), weapons = new Set((entity.system || []).filter(row => kinds.includes(row.kind)).map(row => row.id));
  for (const group of groups) {
    if (!group || typeof group !== 'object' || Array.isArray(group) || Object.keys(group).some(key => !['id', 'label', 'members'].includes(key))
      || typeof group.id !== 'string' || !/^[a-z0-9-]+$/.test(group.id) || ids.has(group.id)
      || typeof group.label !== 'string' || !group.label.trim() || !Array.isArray(group.members) || !group.members.length
      || new Set(group.members).size !== group.members.length || group.members.some(id => !weapons.has(id)))
      throw new Error('Use distinct group ids, a label key and unique actual weapon System members.');
    ids.add(group.id);
  }
}
export function inspectWeaponGroups(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path), entity = inspection.resolved.value;
  const groups = structuredClone(entity.weapons_console?.group || []);
  validateWeaponGroups(groups, entity);
  return { ...inspection, groups, weapons: (entity.system || []).filter(row => kinds.includes(row.kind)),
    capable: !!entity.weapons_console,
    inherited: inspection.fields.some(row => (row.path === 'weapons_console.group' || row.path.startsWith('weapons_console.group.')) && row.owner !== path) };
}
export function guardWeaponGroupReferences(_before, after) {
  validateWeaponGroups(after.weapons_console?.group || [], after);
}
const ordered = value => Array.isArray(value) ? value.map(ordered) : value && typeof value === 'object'
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])])) : value;
function writeGroups(source, groups) {
  const own = parse(source), nl = source.includes('\r\n') ? '\r\n' : '\n';
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, body: match.index + match[0].length, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'weapons_console.group' || row.name.startsWith('weapons_console.group.'))
    .map(row => ({ start: row.start, end: headers.find(next => next.start > row.start)?.start ?? source.length }));
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  if (!spans.length && own.weapons_console?.group !== undefined) {
    const parent = headers.find(row => row.name === 'weapons_console');
    const end = parent && (headers.find(next => next.start > parent.start)?.start ?? source.length);
    const empty = parent && /^[ \t]*group[ \t]*=[ \t]*\[[ \t]*\][ \t]*(?:#[^\r\n]*)?(?:\r?\n|$)/m.exec(source.slice(parent.body, end));
    if (!empty) throw new Error('Expand inline group definitions into ordinary TOML tables before editing.');
    after = source.slice(0, parent.body + empty.index) + source.slice(parent.body + empty.index + empty[0].length);
  }
  const text = groups.length ? stringify({ weapons_console: { group: groups } }).trim() : '[weapons_console]\ngroup = []';
  if (!groups.length && parse(after).weapons_console) {
    const parent = /^[ \t]*\[weapons_console\][ \t]*(?:#.*)?(?:\r?\n|$)/m.exec(after);
    if (parent) after = after.slice(0, parent.index + parent[0].length) + 'group = []' + nl + after.slice(parent.index + parent[0].length);
    else after += nl + text.replace(/\n/g, nl) + nl;
  } else after += (after.endsWith(nl + nl) ? '' : after.endsWith(nl) ? nl : nl + nl) + text.replace(/\n/g, nl) + nl;
  const parsed = parse(after), without = value => {
    const copy = structuredClone(value); if (copy.weapons_console) { delete copy.weapons_console.group; if (!Object.keys(copy.weapons_console).length) delete copy.weapons_console; }
    return JSON.stringify(ordered(copy));
  };
  if (JSON.stringify(parsed.weapons_console?.group) !== JSON.stringify(groups) || without(own) !== without(parsed))
    throw new Error('Expand group source into ordinary TOML tables before editing.');
  return after;
}
export function prepareWeaponGroupsOperation(draft, dependencies, operation) {
  const inspection = inspectWeaponGroups(draft, dependencies, operation.path);
  if (!inspection.capable) throw new Error('Choose actual Weapons equipment before preparing groups.');
  if (inspection.inherited && !operation.materialise) throw new Error('Explicitly materialise inherited weapon groups before editing.');
  validateWeaponGroups(operation.value, inspection.resolved.value);
  const before = draft.read(operation.path), after = writeGroups(before, operation.value);
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
export async function applyWeaponGroupsOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => prepareWeaponGroupsOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectWeaponGroups(candidate, effective, selected.path) });
}
