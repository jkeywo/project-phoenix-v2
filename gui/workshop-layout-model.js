import { PANEL_KIND } from './dock-layout-model.js';
import { addMigrationPanel } from './dock-layout-migration.js';
import { createDockLayoutHistory } from './dock-layout-history.js';

export const WORKSHOP_LAYOUT_VERSION = 11;
// id, introduction version, migration order, preferred target, default column, kind.
// Vocabulary order is record order, independently of migration placement order.
const introductions = Object.freeze([
  ['files', 2, 0, '', 0, PANEL_KIND.TOOL],
  ['source', 2, 0, '', 1, PANEL_KIND.DOCUMENT],
  ['inspector', 2, 0, '', 2, PANEL_KIND.TOOL],
  ['add', 2, 0, '', 2, PANEL_KIND.TOOL],
  ['recovery', 2, 0, '', 2, PANEL_KIND.TOOL],
  ['findings', 3, 1, 'source', 1, PANEL_KIND.TOOL],
  ['feedback', 3, 2, 'source', 1, PANEL_KIND.TOOL],
  ['dependencies', 3, 0, 'files', 0, PANEL_KIND.TOOL],
  ['settings', 3, 3, 'inspector', 2, PANEL_KIND.TOOL],
  ['models', 4, 4, 'inspector', 2, PANEL_KIND.TOOL],
  ['model-preview', 4, 5, 'source', 1, PANEL_KIND.DOCUMENT],
  ['sound', 4, 6, 'inspector', 2, PANEL_KIND.TOOL],
  ['changes', 5, 7, 'files', 0, PANEL_KIND.TOOL],
  ['definitions', 6, 8, 'inspector', 2, PANEL_KIND.TOOL],
  ['composition', 7, 9, 'files', 0, PANEL_KIND.TOOL],
  ['entity', 8, 10, 'inspector', 2, PANEL_KIND.TOOL],
  ['presets', 9, 11, 'files', 0, PANEL_KIND.TOOL],
  ['scripts', 10, 12, 'source', 1, PANEL_KIND.DOCUMENT],
  ['localisation', 11, 13, 'dependencies', 0, PANEL_KIND.TOOL],
]);
const records = introductions.map(([id, since, order, target, , kind]) => ({
  id, since, kind, ...(target ? { migration: { order, target } } : {}),
}));
const historicalDefault = version => ({ version,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: ['files', 'source', 'inspector'].map((active, column) => ({ type: 'tabs', active,
      tabs: introductions.filter(([, introduced, , , home]) => introduced <= version && home === column).map(([id]) => id) })) },
  floats: [], closed: [], selected: 'source' });
export const defaultWorkshopLayout = () => historicalDefault(WORKSHOP_LAYOUT_VERSION);

// Version 1 held `add` and `recovery` as fixed chrome rather than as placements.
// They are rehomed first, so a v1 tree ends up where a v2 tree of the same shape
// would have — and a panel that tree explicitly closed is left closed.
function rehomeVersionOne(state, from, value, generation) {
  if (from !== 1) return state;
  const legacy = generation.model;
  const preservedClosed = Array.isArray(value.closed)
    ? value.closed.filter(panel => legacy.panels.includes(panel)) : [];
  const rehomed = ['add', 'recovery'].reduce(
    (current, panel) => addMigrationPanel(current, panel, 'inspector', legacy, preservedClosed),
    { ...state, version: 2 });
  return { ...rehomed, version: WORKSHOP_LAYOUT_VERSION };
}

const { current: base, registry, migrate } = createDockLayoutHistory({
  version: WORKSHOP_LAYOUT_VERSION, panels: records, aliases: { 1: 2 },
  defaultLayout: historicalDefault, rehome: rehomeVersionOne,
});
export const WORKSHOP_PANEL_REGISTRY = registry;
export const WORKSHOP_PANELS = base.panels;
export const workshopLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeWorkshopLayout = migrate;
export const selectWorkshopPanel = base.select;
export const dockWorkshopPanel = base.dock;
export const floatWorkshopPanel = base.float;
export const moveWorkshopFloat = base.moveFloat;
export const closeWorkshopPanel = base.close;
export const reopenWorkshopPanel = base.reopen;
