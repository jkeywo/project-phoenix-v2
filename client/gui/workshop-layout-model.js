import { createDockLayoutModel } from './dock-layout-model.js';

export const WORKSHOP_LAYOUT_VERSION = 3;
export const WORKSHOP_PANEL_REGISTRY = Object.freeze([
  Object.freeze({ id: 'files' }), Object.freeze({ id: 'source' }), Object.freeze({ id: 'inspector' }),
  Object.freeze({ id: 'add' }), Object.freeze({ id: 'recovery' }), Object.freeze({ id: 'findings' }),
  Object.freeze({ id: 'feedback' }), Object.freeze({ id: 'dependencies' }), Object.freeze({ id: 'settings' }),
]);
export const WORKSHOP_PANELS = Object.freeze(WORKSHOP_PANEL_REGISTRY.map(panel => panel.id));
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
const LEGACY_PANELS = Object.freeze(['files', 'source', 'inspector', 'add', 'recovery']);
const legacyDefault = () => ({ version: 2,
  root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
    children: [group(['files']), group(['source']), group(['inspector', 'add', 'recovery'], 'inspector')] },
  floats: [], closed: [], selected: 'source' });
export function defaultWorkshopLayout() {
  return { version: WORKSHOP_LAYOUT_VERSION,
    root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
      children: [group(['files', 'dependencies'], 'files'), group(['source', 'findings', 'feedback'], 'source'),
        group(['inspector', 'add', 'recovery', 'settings'], 'inspector')] },
    floats: [], closed: [], selected: 'source' };
}
const base = createDockLayoutModel({ version: WORKSHOP_LAYOUT_VERSION, panels: WORKSHOP_PANELS,
  defaultLayout: defaultWorkshopLayout, compatibleVersions: [1, 2, WORKSHOP_LAYOUT_VERSION] });
const legacy = createDockLayoutModel({ version: 2, panels: LEGACY_PANELS,
  defaultLayout: legacyDefault, compatibleVersions: [1, 2] });

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
  if (![1, 2].includes(value?.version)) return base.normalize(value, bounds);
  const normalized = legacy.normalize(value, bounds);
  const preservedClosed = Array.isArray(value.closed) ? value.closed.filter(panel => LEGACY_PANELS.includes(panel)) : [];
  let migrated = { ...normalized, version: WORKSHOP_LAYOUT_VERSION,
    closed: [...normalized.closed, 'dependencies', 'findings', 'feedback', 'settings'] };
  const previousActives = activePanels(normalized.root);
  if (value.version === 1) {
    const versionTwo = { ...migrated, version: 2 };
    migrated = ['add', 'recovery'].reduce(
      (state, panel) => addMigrationPanel(state, panel, 'inspector', legacy, preservedClosed), versionTwo);
    migrated.version = WORKSHOP_LAYOUT_VERSION;
  }
  migrated = base.normalize(migrated, bounds);
  for (const [panel, preferred] of [['dependencies', 'files'], ['findings', 'source'],
    ['feedback', 'source'], ['settings', 'inspector']]) {
    migrated = addMigrationPanel(migrated, panel, preferred, base);
  }
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
