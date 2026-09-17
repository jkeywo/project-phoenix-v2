import { createDockLayoutModel } from './dock-layout-model.js';
import { createDockLayoutMigration } from './dock-layout-migration.js';

export const LIVE_LAYOUT_VERSION = 2;
export const LIVE_PANELS = Object.freeze(['roster', 'readiness', 'join', 'manual-save',
  'mission', 'comms', 'activity', 'journal', 'session-history']);
const V1_PANELS = Object.freeze(['roster', 'readiness', 'join', 'manual-save']);
/** Panels registered after version 1, with the group each joins on migration.
 *
 * Comms opens a group BELOW the readiness panels rather than joining them,
 * because the three log views and the session history are one reading surface
 * on this desk and always were: they shared one region behind one tab strip.
 * Migration preserves that relationship; docking is free to dissolve it. */
const ADDED_IN_V2 = Object.freeze([
  ['mission', 'roster', 'tab'],
  ['comms', 'roster', 'bottom'],
  ['activity', 'comms', 'tab'],
  ['journal', 'comms', 'tab'],
  ['session-history', 'comms', 'tab'],
]);
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
const v1Default = () => ({ version: 1,
  root: group(['roster', 'readiness', 'join', 'manual-save'], 'roster'),
  floats: [], closed: [], selected: 'roster' });
export function defaultLiveLayout() {
  return { version: LIVE_LAYOUT_VERSION,
    root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
      group(['roster', 'readiness', 'join', 'manual-save', 'mission'], 'roster'),
      group(['comms', 'activity', 'journal', 'session-history'], 'comms'),
    ] },
    floats: [], closed: [], selected: 'roster' };
}
const base = createDockLayoutModel({
  version: LIVE_LAYOUT_VERSION, panels: LIVE_PANELS, defaultLayout: defaultLiveLayout,
  compatibleVersions: [LIVE_LAYOUT_VERSION],
});
const v1 = createDockLayoutModel({ version: 1, panels: V1_PANELS, defaultLayout: v1Default,
  compatibleVersions: [1] });
const migrate = createDockLayoutMigration({
  version: LIVE_LAYOUT_VERSION, current: base,
  generations: [
    { version: 1, model: v1, added: [] },
    { version: LIVE_LAYOUT_VERSION, model: base, added: ADDED_IN_V2 },
  ],
});
export const liveLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeLiveLayout = migrate;
