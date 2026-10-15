import { parse } from 'smol-toml';
import { textMembers, tomlBasicString } from './workshop-definitions.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const worldPath = value => typeof value === 'string' && value.length <= 512
  && value.startsWith('assets/worlds/') && value.endsWith('.toml')
  && !/[\\\x00-\x1f\x7f]/.test(value) && value.split('/').every(part => part && part !== '.' && part !== '..');
const identity = value => typeof value === 'string' && value.trim() && new TextEncoder().encode(value).length <= 512
  && !/[\x00-\x1f\x7f]/.test(value);
const finitePoint = value => Array.isArray(value) && value.length === 3
  && value.every(item => typeof item === 'number' && Number.isFinite(Math.fround(item)));
const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value)
  && Object.keys(value).every(key => keys.includes(key));

function sources(draft, dependencies) {
  const files = { ...(dependencies?.base_files || {}) };
  for (const pack of dependencies?.packs || []) Object.assign(files, pack.files || {});
  Object.assign(files, textMembers(draft));
  return files;
}
export function inspectWorldRoutes(draft, dependencies = {}, path) {
  const files = sources(draft, dependencies);
  const manifest = draft.read(draft.kind === 'mod' ? 'scenarios.toml' : 'assets/scenarios.toml');
  const roots = new Set(typeof manifest === 'string' ? (parse(manifest).scenario || []).map(row => row.world) : []);
  const worlds = Object.entries(files).filter(([file, source]) => worldPath(file) && typeof source === 'string')
    .map(([file, source]) => ({ path: file, anchors: Object.entries(parse(source).anchors || {})
      .filter(([, point]) => finitePoint(point)).map(([name, point]) => ({ name, point })) }))
    .sort((a, b) => a.path.localeCompare(b.path));
  const selected = typeof draft.read(path) === 'string' && worldPath(path) ? parse(draft.read(path)) : null;
  return { path, editable: Boolean(selected && roots.has(path)),
    roots: worlds.filter(row => roots.has(row.path) && typeof draft.read(row.path) === 'string'),
    worlds, routes: structuredClone(selected?.transit_destination || []) };
}
function validateRoutes(rows, inspection) {
  if (!Array.isArray(rows) || rows.length > 128) throw new Error('World routes support at most 128 entries.');
  const ids = new Set(), points = new Set();
  for (const row of rows) {
    if (!closed(row, ['id', 'label', 'target']) || !identity(row.id) || !identity(row.label) || ids.has(row.id))
      throw new Error('Choose a unique bounded route ID and label string key.');
    ids.add(row.id);
    const target = row.target;
    if (!closed(target, target?.kind === 'base' ? ['kind', 'anchor'] : ['kind', 'path', 'anchor'])
      || !['base', 'layer'].includes(target.kind) || !identity(target.anchor)
      || (target.kind === 'layer' && !worldPath(target.path))) throw new Error('Choose an actual World and arrival anchor.');
    const world = inspection.worlds.find(world => world.path === (target.kind === 'base' ? inspection.path : target.path));
    const anchor = world?.anchors.find(anchor => anchor.name === target.anchor);
    if (!anchor) throw new Error('The selected World must contain that finite arrival anchor.');
    const key = JSON.stringify([Math.fround(anchor.point[0]), Math.fround(anchor.point[2])]);
    if (points.has(key)) throw new Error('World route arrival anchors must have distinct x/z coordinates.');
    points.add(key);
  }
}
const targetSource = target => target.kind === 'base'
  ? `{ kind = "base", anchor = ${tomlBasicString(target.anchor)} }`
  : `{ kind = "layer", path = ${tomlBasicString(target.path)}, anchor = ${tomlBasicString(target.anchor)} }`;

/** The existing runtime TOML editor retains exact unrelated source. The same
 * accepted-source transaction owns history, validation, export and Test. */
export async function applyWorldRouteOperation({ draft, dependencies, provider, runtime, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, dependencies, provider, runtime, current,
    stale: 'stale-world-route-operation',
    prepare: async (captured, effective) => {
      const inspection = inspectWorldRoutes(captured, effective, selected.path);
      if (!inspection.editable) throw new Error('Edit a selectable root World in this draft.');
      const rows = inspection.routes, index = rows.findIndex(row => row.id === selected.original);
      let edits;
      if (selected.type === 'route-remove') {
        if (index < 0) throw new Error('The selected route has changed.');
        rows.splice(index, 1); edits = [{ op: 'remove', path: ['transit_destination', index] }];
      } else {
        const row = { id: selected.id, label: selected.label, target: selected.target };
        if (selected.type === 'route-add') {
          rows.push(row);
          edits = [{ op: 'append_table', path: ['transit_destination'], fields: [
            ['id', tomlBasicString(row.id)], ['label', tomlBasicString(row.label)], ['target', targetSource(row.target)],
          ] }];
        } else if (selected.type === 'route-update' && index >= 0) {
          const previous = rows[index]; rows[index] = row;
          edits = ['id', 'label'].map(key => ({ op: 'put', path: ['transit_destination', index, key], value_source: tomlBasicString(row[key]) }));
          for (const [key, value] of Object.entries(row.target))
            edits.push({ op: 'put', path: ['transit_destination', index, 'target', key], value_source: tomlBasicString(value) });
          if (previous.target?.path !== undefined && row.target.kind === 'base')
            edits.push({ op: 'remove', path: ['transit_destination', index, 'target', 'path'] });
        } else throw new Error('The selected route has changed.');
      }
      validateRoutes(rows, inspection);
      const before = captured.read(selected.path);
      const after = await runtime.compose(textMembers(captured), { document_path: selected.path, expected_source: before, edits });
      return [{ path: selected.path, before, after }];
    }, inspect: (candidate, effective) => inspectWorldRoutes(candidate, effective, selected.path) });
}
