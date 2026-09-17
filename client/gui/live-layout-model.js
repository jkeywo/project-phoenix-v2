import { createDockLayoutModel } from './dock-layout-model.js';

export const LIVE_LAYOUT_VERSION = 1;
export const LIVE_PANELS = Object.freeze(['roster', 'readiness', 'join', 'manual-save']);
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
export function defaultLiveLayout() {
  return { version: LIVE_LAYOUT_VERSION,
    root: group(['roster', 'readiness', 'join', 'manual-save'], 'roster'),
    floats: [], closed: [], selected: 'roster' };
}
export const liveLayoutModel = createDockLayoutModel({
  version: LIVE_LAYOUT_VERSION, panels: LIVE_PANELS, defaultLayout: defaultLiveLayout,
});
export const normalizeLiveLayout = liveLayoutModel.normalize;
