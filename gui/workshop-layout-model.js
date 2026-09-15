import { createDockLayoutModel } from './dock-layout-model.js';

export const WORKSHOP_LAYOUT_VERSION = 2;
export const WORKSHOP_PANEL_REGISTRY = Object.freeze([
  Object.freeze({ id: 'files' }), Object.freeze({ id: 'source' }), Object.freeze({ id: 'inspector' }),
  Object.freeze({ id: 'add' }), Object.freeze({ id: 'recovery' }),
]);
export const WORKSHOP_PANELS = Object.freeze(WORKSHOP_PANEL_REGISTRY.map(panel => panel.id));
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
export function defaultWorkshopLayout() {
  return { version: WORKSHOP_LAYOUT_VERSION,
    root: { type: 'split', axis: 'horizontal', sizes: [22, 56, 22],
      children: [group(['files']), group(['source']), group(['inspector', 'add', 'recovery'], 'inspector')] },
    floats: [], closed: [], selected: 'source' };
}
const base = createDockLayoutModel({ version: WORKSHOP_LAYOUT_VERSION, panels: WORKSHOP_PANELS,
  defaultLayout: defaultWorkshopLayout, compatibleVersions: [1, WORKSHOP_LAYOUT_VERSION] });

function firstVisible(node, floats = []) {
  if (node?.type === 'tabs') return node.active;
  if (node?.type === 'split') return node.children.map(child => firstVisible(child)).find(Boolean);
  return floats[0]?.panel || null;
}

function migrate(value, bounds) {
  const normalized = base.normalize(value, bounds);
  if (value?.version !== 1) return normalized;
  const target = !normalized.closed.includes('inspector') ? 'inspector'
    : firstVisible(normalized.root, normalized.floats);
  if (!target) return defaultWorkshopLayout();
  const migrated = ['add', 'recovery'].reduce((state, panel) => base.dock(state, panel, target, 'tab'), normalized);
  migrated.selected = normalized.selected; return migrated;
}
export const workshopLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeWorkshopLayout = migrate;
export const selectWorkshopPanel = base.select;
export const dockWorkshopPanel = base.dock;
export const floatWorkshopPanel = base.float;
export const moveWorkshopFloat = base.moveFloat;
export const closeWorkshopPanel = base.close;
export const reopenWorkshopPanel = base.reopen;
