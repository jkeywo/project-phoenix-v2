import { createDockLayoutModel, PANEL_KIND } from './dock-layout-model.js';

export const WORKSHOP_LAYOUT_VERSION = 4;
const tool = id => Object.freeze({ id, kind: PANEL_KIND.TOOL });
const document_ = id => Object.freeze({ id, kind: PANEL_KIND.DOCUMENT });
export const WORKSHOP_PANEL_REGISTRY = Object.freeze([
  tool('files'), document_('source'), tool('inspector'),
  tool('add'), tool('recovery'), tool('findings'),
  tool('feedback'), tool('dependencies'), tool('settings'),
  tool('models'), document_('model-preview'), tool('sound'),
]);
export const WORKSHOP_PANELS = Object.freeze(WORKSHOP_PANEL_REGISTRY.map(panel => panel.id));
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
const LEGACY_PANELS = Object.freeze(['files', 'source', 'inspector', 'add', 'recovery']);
const V3_PANELS = Object.freeze([...LEGACY_PANELS, 'findings', 'feedback', 'dependencies', 'settings']);
// Panels registered after a stored version, with the group each joins on migration.
const ADDED_IN_V3 = Object.freeze([['dependencies', 'files'], ['findings', 'source'],
  ['feedback', 'source'], ['settings', 'inspector']]);
const ADDED_IN_V4 = Object.freeze([['models', 'inspector'], ['model-preview', 'source'], ['sound', 'inspector']]);
const legacyDefault = () => ({ version: 2,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files']), group(['source']), group(['inspector', 'add', 'recovery'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
const v3Default = () => ({ version: 3,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files', 'dependencies'], 'files'), group(['source', 'findings', 'feedback'], 'source'),
      group(['inspector', 'add', 'recovery', 'settings'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
export function defaultWorkshopLayout() {
  return { version: WORKSHOP_LAYOUT_VERSION,
    root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
      children: [group(['files', 'dependencies'], 'files'),
        group(['source', 'findings', 'feedback', 'model-preview'], 'source'),
        group(['inspector', 'add', 'recovery', 'settings', 'models', 'sound'], 'inspector')] },
    floats: [], closed: [], selected: 'source' };
}
const base = createDockLayoutModel({ version: WORKSHOP_LAYOUT_VERSION, panels: WORKSHOP_PANEL_REGISTRY,
  defaultLayout: defaultWorkshopLayout, compatibleVersions: [1, 2, 3, WORKSHOP_LAYOUT_VERSION] });
const legacy = createDockLayoutModel({ version: 2, panels: LEGACY_PANELS,
  defaultLayout: legacyDefault, compatibleVersions: [1, 2] });
const v3 = createDockLayoutModel({ version: 3, panels: V3_PANELS,
  defaultLayout: v3Default, compatibleVersions: [3] });

function firstVisible(node, floats = []) {
  if (node?.type === 'tabs') return node.active;
  if (node?.type === 'split') return node.children.map(child => firstVisible(child)).find(Boolean);
  return floats[0]?.panel || null;
}

function containsPanel(node, panel) {
  if (node?.type === 'tabs') return node.tabs.includes(panel);
  return node?.type === 'split' && node.children.some(child => containsPanel(child, panel));
}

function addMigrationPanel(state, panel, preferred, model, preservedClosed = []) {
  if (!state.closed.includes(panel) || preservedClosed.includes(panel)) return state;
  const target = containsPanel(state.root, preferred) ? preferred : firstVisible(state.root);
  if (target) return model.dock(state, panel, target, 'tab');
  return state.floats.length ? model.reopen(state, panel) : state;
}

function activePanels(node, out = new Map()) {
  if (node?.type === 'tabs') node.tabs.forEach(panel => out.set(panel, node.active));
  else if (node?.type === 'split') node.children.forEach(child => activePanels(child, out));
  return out;
}

function restoreActives(node, previous) {
  if (node?.type === 'tabs') {
    const active = node.tabs.map(panel => previous.get(panel)).find(panel => node.tabs.includes(panel));
    if (active) node.active = active;
  } else if (node?.type === 'split') node.children.forEach(child => restoreActives(child, previous));
}

function migrate(value, bounds) {
  const stored = value?.version;
  if (![1, 2, 3].includes(stored)) return base.normalize(value, bounds);
  // Sanitize against the vocabulary the stored version actually had, so a panel
  // registered later can never be read back out of an older tree.
  const priorModel = stored === 3 ? v3 : legacy;
  const priorPanels = stored === 3 ? V3_PANELS : LEGACY_PANELS;
  const normalized = priorModel.normalize(value, bounds);
  const added = stored === 3 ? [...ADDED_IN_V4] : [...ADDED_IN_V3, ...ADDED_IN_V4];
  let migrated = { ...normalized, version: WORKSHOP_LAYOUT_VERSION,
    closed: [...normalized.closed, ...added.map(([panel]) => panel)] };
  const previousActives = activePanels(normalized.root);
  if (stored === 1) {
    const preservedClosed = Array.isArray(value.closed) ? value.closed.filter(panel => priorPanels.includes(panel)) : [];
    const versionTwo = { ...migrated, version: 2 };
    migrated = ['add', 'recovery'].reduce(
      (state, panel) => addMigrationPanel(state, panel, 'inspector', legacy, preservedClosed), versionTwo);
    migrated.version = WORKSHOP_LAYOUT_VERSION;
  }
  migrated = base.normalize(migrated, bounds);
  for (const [panel, preferred] of added) migrated = addMigrationPanel(migrated, panel, preferred, base);
  restoreActives(migrated.root, previousActives);
  migrated.selected = normalized.selected;
  return migrated;
}
export const workshopLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeWorkshopLayout = migrate;
export const selectWorkshopPanel = base.select;
export const dockWorkshopPanel = base.dock;
export const floatWorkshopPanel = base.float;
export const moveWorkshopFloat = base.moveFloat;
export const closeWorkshopPanel = base.close;
export const reopenWorkshopPanel = base.reopen;
