import { createDockLayoutModel, PANEL_KIND } from './dock-layout-model.js';

export const WORKSHOP_TEST_LAYOUT_VERSION = 1;
export const WORKSHOP_TEST_PANEL_REGISTRY = Object.freeze([
  Object.freeze({ id: 'test-controls', kind: PANEL_KIND.TOOL }),
  Object.freeze({ id: 'test-viewscreen', kind: PANEL_KIND.DOCUMENT }),
]);
export const WORKSHOP_TEST_PANELS = Object.freeze(
  WORKSHOP_TEST_PANEL_REGISTRY.map(panel => panel.id),
);

const group = panel => ({ type: 'tabs', tabs: [panel], active: panel });

export function defaultWorkshopTestLayout() {
  return {
    version: WORKSHOP_TEST_LAYOUT_VERSION,
    root: {
      type: 'split', axis: 'horizontal', sizes: [30, 70],
      children: [group('test-controls'), group('test-viewscreen')],
    },
    floats: [], closed: [], selected: 'test-viewscreen',
  };
}

export const workshopTestLayoutModel = createDockLayoutModel({
  version: WORKSHOP_TEST_LAYOUT_VERSION,
  panels: WORKSHOP_TEST_PANEL_REGISTRY,
  defaultLayout: defaultWorkshopTestLayout,
});

export const normalizeWorkshopTestLayout = workshopTestLayoutModel.normalize;
