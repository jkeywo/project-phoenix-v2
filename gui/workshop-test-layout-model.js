import { createDockLayoutModel, PANEL_KIND } from './dock-layout-model.js';

export const WORKSHOP_TEST_LAYOUT_VERSION = 2;
export const WORKSHOP_TEST_PANEL_REGISTRY = Object.freeze([
  Object.freeze({ id: 'test-controls', kind: PANEL_KIND.TOOL }),
  Object.freeze({ id: 'test-viewscreen', kind: PANEL_KIND.DOCUMENT }),
  Object.freeze({ id: 'test-trace', kind: PANEL_KIND.TOOL }),
]);
export const WORKSHOP_TEST_PANELS = Object.freeze(
  WORKSHOP_TEST_PANEL_REGISTRY.map(panel => panel.id),
);

const group = panel => ({ type: 'tabs', tabs: [panel], active: panel });

export function defaultWorkshopTestLayout() {
  return {
    version: WORKSHOP_TEST_LAYOUT_VERSION,
    root: {
      type: 'split', axis: 'horizontal', sizes: [25, 50, 25],
      children: [group('test-controls'), group('test-viewscreen'), group('test-trace')],
    },
    floats: [], closed: [], selected: 'test-viewscreen',
  };
}

const baseWorkshopTestLayoutModel = createDockLayoutModel({
  version: WORKSHOP_TEST_LAYOUT_VERSION,
  panels: WORKSHOP_TEST_PANEL_REGISTRY,
  defaultLayout: defaultWorkshopTestLayout,
  compatibleVersions: [1, WORKSHOP_TEST_LAYOUT_VERSION],
});

function normalize(value, bounds) {
  let normalized = baseWorkshopTestLayoutModel.normalize(value, bounds);
  if (value?.version !== 1) return normalized;
  const selected = normalized.selected;
  normalized = baseWorkshopTestLayoutModel.reopen(normalized, 'test-trace');
  normalized = baseWorkshopTestLayoutModel.dock(normalized, 'test-trace', 'test-viewscreen', 'right');
  return baseWorkshopTestLayoutModel.settle({ ...normalized, selected }, bounds);
}

export const workshopTestLayoutModel = Object.freeze({ ...baseWorkshopTestLayoutModel, normalize });

export const normalizeWorkshopTestLayout = normalize;
