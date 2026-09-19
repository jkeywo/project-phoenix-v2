import { createDockLayoutModel, PANEL_KIND } from './dock-layout-model.js';
import { addMigrationPanel, createDockLayoutMigration } from './dock-layout-migration.js';

export const WORKSHOP_LAYOUT_VERSION = 8;
const tool = id => Object.freeze({ id, kind: PANEL_KIND.TOOL });
const documentPanel = id => Object.freeze({ id, kind: PANEL_KIND.DOCUMENT });
export const WORKSHOP_PANEL_REGISTRY = Object.freeze([
  tool('files'), documentPanel('source'), tool('inspector'),
  tool('add'), tool('recovery'), tool('findings'),
  tool('feedback'), tool('dependencies'), tool('settings'),
  tool('models'), documentPanel('model-preview'), tool('sound'),
  tool('changes'), tool('definitions'), tool('composition'), tool('entity'),
]);
export const WORKSHOP_PANELS = Object.freeze(WORKSHOP_PANEL_REGISTRY.map(panel => panel.id));
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
const LEGACY_PANELS = Object.freeze(['files', 'source', 'inspector', 'add', 'recovery']);
const V3_PANELS = Object.freeze([...LEGACY_PANELS, 'findings', 'feedback', 'dependencies', 'settings']);
// Panels registered after a stored version, with the group each joins on migration.
const ADDED_IN_V3 = Object.freeze([['dependencies', 'files'], ['findings', 'source'],
  ['feedback', 'source'], ['settings', 'inspector']]);
const ADDED_IN_V4 = Object.freeze([['models', 'inspector'], ['model-preview', 'source'], ['sound', 'inspector']]);
const V4_PANELS = Object.freeze([...V3_PANELS, 'models', 'model-preview', 'sound']);
/** Panels registered after version 4. The workspace changes view reads the same
 * draft the file list does — what has been added, removed, renamed or modified
 * against the source this draft was imported as — so it joins that column
 * (issue #1471). */
const ADDED_IN_V5 = Object.freeze([['changes', 'files']]);
const V5_PANELS = Object.freeze([...V4_PANELS, 'changes']);
/** Panels registered after version 5. Faction and complexity definitions are a
 * specialised form over the same runtime the inspector reads, so it joins the
 * inspector's column beside the model form (issue #1474). */
const ADDED_IN_V6 = Object.freeze([['definitions', 'inspector']]);
const V6_PANELS = Object.freeze([...V5_PANELS, 'definitions']);
/** Panels registered after version 6. World composition reads the draft's
 * member set the way the changes view does — which members exist, which
 * reference which — so it joins that column beside it (issue #1475). */
const ADDED_IN_V7 = Object.freeze([['composition', 'files']]);
const V7_PANELS = Object.freeze([...V6_PANELS, 'composition']);
/** Panels registered after version 7. Entity template and fragment composition
 * is a specialised form over the same runtime the inspector reads, so it joins
 * the inspector's column beside the definition forms (issue #1476). #1481 will
 * extend that panel rather than add another, because it authors the same
 * document. */
const ADDED_IN_V8 = Object.freeze([['entity', 'inspector']]);
const legacyDefault = () => ({ version: 2,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files']), group(['source']), group(['inspector', 'add', 'recovery'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
const v4Default = () => ({ version: 4,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files', 'dependencies'], 'files'),
      group(['source', 'findings', 'feedback', 'model-preview'], 'source'),
      group(['inspector', 'add', 'recovery', 'settings', 'models', 'sound'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
const v3Default = () => ({ version: 3,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files', 'dependencies'], 'files'), group(['source', 'findings', 'feedback'], 'source'),
      group(['inspector', 'add', 'recovery', 'settings'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
const v5Default = () => ({ version: 5,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files', 'dependencies', 'changes'], 'files'),
      group(['source', 'findings', 'feedback', 'model-preview'], 'source'),
      group(['inspector', 'add', 'recovery', 'settings', 'models', 'sound'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
const v6Default = () => ({ version: 6,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files', 'dependencies', 'changes'], 'files'),
      group(['source', 'findings', 'feedback', 'model-preview'], 'source'),
      group(['inspector', 'add', 'recovery', 'settings', 'models', 'sound', 'definitions'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
const v7Default = () => ({ version: 7,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files', 'dependencies', 'changes', 'composition'], 'files'),
      group(['source', 'findings', 'feedback', 'model-preview'], 'source'),
      group(['inspector', 'add', 'recovery', 'settings', 'models', 'sound', 'definitions'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
export function defaultWorkshopLayout() {
  return { version: WORKSHOP_LAYOUT_VERSION,
    root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
      children: [group(['files', 'dependencies', 'changes', 'composition'], 'files'),
        group(['source', 'findings', 'feedback', 'model-preview'], 'source'),
        group(['inspector', 'add', 'recovery', 'settings', 'models', 'sound', 'definitions', 'entity'], 'inspector')] },
    floats: [], closed: [], selected: 'source' };
}
const base = createDockLayoutModel({ version: WORKSHOP_LAYOUT_VERSION, panels: WORKSHOP_PANEL_REGISTRY,
  defaultLayout: defaultWorkshopLayout, compatibleVersions: [WORKSHOP_LAYOUT_VERSION] });
const legacy = createDockLayoutModel({ version: 2, panels: LEGACY_PANELS,
  defaultLayout: legacyDefault, compatibleVersions: [1, 2] });
const v3 = createDockLayoutModel({ version: 3, panels: V3_PANELS,
  defaultLayout: v3Default, compatibleVersions: [3] });
const v4 = createDockLayoutModel({ version: 4, panels: V4_PANELS,
  defaultLayout: v4Default, compatibleVersions: [4] });
const v5 = createDockLayoutModel({ version: 5, panels: V5_PANELS,
  defaultLayout: v5Default, compatibleVersions: [5] });
const v6 = createDockLayoutModel({ version: 6, panels: V6_PANELS,
  defaultLayout: v6Default, compatibleVersions: [6] });
const v7 = createDockLayoutModel({ version: 7, panels: V7_PANELS,
  defaultLayout: v7Default, compatibleVersions: [7] });

// Version 1 held `add` and `recovery` as fixed chrome rather than as placements.
// They are rehomed first, so a v1 tree ends up where a v2 tree of the same shape
// would have — and a panel that tree explicitly closed is left closed.
function rehomeVersionOne(state, from, value) {
  if (from !== 1) return state;
  const preservedClosed = Array.isArray(value.closed)
    ? value.closed.filter(panel => LEGACY_PANELS.includes(panel)) : [];
  const rehomed = ['add', 'recovery'].reduce(
    (current, panel) => addMigrationPanel(current, panel, 'inspector', legacy, preservedClosed),
    { ...state, version: 2 });
  return { ...rehomed, version: WORKSHOP_LAYOUT_VERSION };
}

const migrate = createDockLayoutMigration({
  version: WORKSHOP_LAYOUT_VERSION, current: base, rehome: rehomeVersionOne,
  generations: [
    { version: 1, model: legacy, added: [] },
    { version: 2, model: legacy, added: [] },
    { version: 3, model: v3, added: ADDED_IN_V3 },
    { version: 4, model: v4, added: ADDED_IN_V4 },
    { version: 5, model: v5, added: ADDED_IN_V5 },
    { version: 6, model: v6, added: ADDED_IN_V6 },
    { version: 7, model: v7, added: ADDED_IN_V7 },
    { version: WORKSHOP_LAYOUT_VERSION, model: base, added: ADDED_IN_V8 },
  ],
});
export const workshopLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeWorkshopLayout = migrate;
export const selectWorkshopPanel = base.select;
export const dockWorkshopPanel = base.dock;
export const floatWorkshopPanel = base.float;
export const moveWorkshopFloat = base.moveFloat;
export const closeWorkshopPanel = base.close;
export const reopenWorkshopPanel = base.reopen;
