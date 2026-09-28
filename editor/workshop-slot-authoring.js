import { parse } from 'smol-toml';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const quote = value => JSON.stringify(String(value));
const parseWorld = source => parse(source.replace(/^\uFEFF/, ''));
const textMembers = draft => Object.fromEntries(draft.paths().filter(path => /\.(toml|rhai)$/.test(path) && typeof draft.read(path) === 'string')
  .map(path => [path, draft.read(path)]));

export function slotWorlds(draft) {
  return (draft?.paths() || []).filter(path => path.startsWith('assets/worlds/') && path.endsWith('.toml')
    && typeof draft.read(path) === 'string');
}

export function inspectSlots(draft, path) {
  const world = parseWorld(draft.read(path));
  return (world.ship_slot || []).map(slot => ({ id: slot.id, label: slot.label || '',
    ships: (slot.ships || []).map(ship => ship.template_path), default_ship: slot.default_ship,
    unclaimed: slot.unclaimed || 'backfill' }));
}

/** Eligibility is the ordinary Test catalogue's composed runtime answer.
 * Origins describe the winning source, never grant editing rights to it. */
export async function slotHullCatalogue({ draft, provider, runtime, dependencies }) {
  const files = textMembers(draft), project = draft.kind === 'project';
  const source = dependencies || await runtime.dependencies();
  const effective = {}, origins = new Map();
  const add = (members, origin) => {
    for (const [path, text] of Object.entries(members || {})) {
      if (typeof text !== 'string') continue;
      effective[path] = text; origins.set(path, origin);
    }
  };
  if (!project) {
    add(source.base_files, { kind: 'base' });
    for (const pack of source.packs || []) add(pack.files, { kind: 'pack', id: pack.id });
  }
  add(files, { kind: 'draft' });
  const catalog = provider?.test?.catalog ? await provider.test.catalog(files) : await runtime.testCatalog(effective);
  return catalog.ships.filter(path => origins.has(path)).map(path => ({ path, origin: origins.get(path) }));
}

/** Plan semantic changes only. The runtime's exact-source editor owns all TOML
 * spans, quoting, comments and line endings; unchanged fields have no edit. */
export function prepareSlotOperation(draft, path, operation, hulls) {
  const before = draft.read(path);
  if (typeof before !== 'string') throw new Error('workshop.slot.error_world');
  const slots = parseWorld(before).ship_slot || [], existing = inspectSlots(draft, path);
  const index = existing.findIndex(row => row.id === operation.target);
  if (!['add', 'update', 'remove'].includes(operation.type)) throw new Error('workshop.slot.error_operation');
  if (operation.type !== 'add' && index < 0) throw new Error('workshop.slot.error_select');
  const row = operation.row && { ...operation.row, label: operation.row.label || '' };
  if (operation.type !== 'remove') {
    if (!row || !/^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(row.id || '')) throw new Error('workshop.slot.error_id');
    const offered = new Set(hulls.map(hull => hull.path));
    if (!Array.isArray(row.ships) || !row.ships.length || new Set(row.ships).size !== row.ships.length
      || row.ships.some(value => !offered.has(value))) throw new Error('workshop.slot.error_ships');
    if (!row.ships.includes(row.default_ship)) throw new Error('workshop.slot.error_default');
    if (!['backfill', 'absent'].includes(row.unclaimed)) throw new Error('workshop.slot.error_policy');
    if (existing.some((item, i) => (operation.type === 'add' || i !== index) && item.id === row.id)) throw new Error('workshop.slot.error_duplicate');
  }
  const edits = [], append = (path, fields) => edits.push({ op: 'append_table', path,
    fields: Object.entries(fields).map(([key, value]) => [key, quote(value)]) });
  if (operation.type === 'remove') edits.push({ op: 'remove', path: ['ship_slot', index] });
  else if (operation.type === 'add') {
    append(['ship_slot'], { id: row.id, ...(row.label ? { label: row.label } : {}),
      default_ship: row.default_ship, unclaimed: row.unclaimed });
    for (const ship of row.ships) append(['ship_slot', slots.length, 'ships'], { template_path: ship });
  } else {
    const base = ['ship_slot', index], prior = slots[index];
    for (const key of ['id', 'label', 'default_ship', 'unclaimed']) {
      if (row[key] === existing[index][key]) continue;
      edits.push(key === 'label' && !row.label ? { op: 'remove', path: [...base, key] }
        : { op: Object.hasOwn(prior, key) ? 'set' : 'put', path: [...base, key], value_source: quote(row[key]) });
    }
    // Retained offers keep their full authored tables, including labels and
    // comments. Removing in reverse keeps subsequent paths stable.
    for (let i = prior.ships.length - 1; i >= 0; i--) {
      if (!row.ships.includes(prior.ships[i].template_path)) edits.push({ op: 'remove', path: [...base, 'ships', i] });
    }
    for (const ship of row.ships) {
      if (!prior.ships.some(offer => offer.template_path === ship)) append([...base, 'ships'], { template_path: ship });
    }
  }
  return { document_path: path, expected_source: before, edits };
}

export async function applySlotOperation({ draft, provider, runtime, path, operation, current = () => true }) {
  const selected = structuredClone(operation);
  const { report } = await acceptWorkshopChanges({ draft, provider, runtime, current,
    stale: 'workshop.slot.error_stale', refused: 'workshop.slot.error_runtime', prepare: async (captured, dependencies) => {
      const hulls = await slotHullCatalogue({ draft: captured, provider, runtime, dependencies });
      const request = prepareSlotOperation(captured, path, selected, hulls);
      if (!request.edits.length) return [];
      const after = await runtime.edit(request.expected_source, request);
      return [{ path, before: request.expected_source, after }];
    } });
  return report;
}
