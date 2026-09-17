import { createDockLayoutModel, PANEL_KIND } from './dock-layout-model.js';
import { createDockLayoutMigration } from './dock-layout-migration.js';

export const LIVE_LAYOUT_VERSION = 4;
const tool = id => Object.freeze({ id, kind: PANEL_KIND.TOOL });
const documentPanel = id => Object.freeze({ id, kind: PANEL_KIND.DOCUMENT });
export const LIVE_PANEL_REGISTRY = Object.freeze([
  tool('roster'), tool('readiness'), tool('join'), tool('manual-save'),
  tool('mission'), tool('comms'), tool('activity'), tool('journal'), tool('session-history'),
  documentPanel('map'), tool('attention'), tool('workload'), tool('widgets'), tool('health'),
  tool('station'), documentPanel('station-console'),
]);
export const LIVE_PANELS = Object.freeze(LIVE_PANEL_REGISTRY.map(panel => panel.id));
const V1_PANELS = Object.freeze(['roster', 'readiness', 'join', 'manual-save']);
const V2_PANELS = Object.freeze([...V1_PANELS,
  'mission', 'comms', 'activity', 'journal', 'session-history']);
const V3_PANELS = Object.freeze([...V2_PANELS, 'map', 'attention', 'workload', 'widgets', 'health']);
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
/** Panels registered after version 2. The map opens a column beside the
 * workflow panels — it is the surface this desk is arranged around — and the
 * awareness panels join the groups that already hold their kind. */
const ADDED_IN_V3 = Object.freeze([
  ['map', 'roster', 'right'],
  ['attention', 'roster', 'tab'],
  ['workload', 'roster', 'tab'],
  ['widgets', 'roster', 'tab'],
  ['health', 'comms', 'tab'],
]);
/** Panels registered after version 3. The authentic Station console is a
 * document and joins the map — the two surfaces this desk is arranged around —
 * while its pending state and takeover controls are an ordinary tool. */
const ADDED_IN_V4 = Object.freeze([
  ['station', 'roster', 'tab'],
  ['station-console', 'map', 'tab'],
]);
/** The attention region renders connection and recovery banners verbatim and the
 * health panel is the readable table behind them. Neither may be hidden by a
 * role preset (they are absent from GM_ROLE_PRESET_PANEL_IDS) and neither may be
 * closed here either: a Game Master must not be able to hide a failure from
 * themselves, whichever mechanism does the hiding. */
export const LIVE_PINNED_PANELS = Object.freeze(['attention', 'health']);
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
const v1Default = () => ({ version: 1,
  root: group(['roster', 'readiness', 'join', 'manual-save'], 'roster'),
  floats: [], closed: [], selected: 'roster' });
const v2Default = () => ({ version: 2,
  root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
    group(['roster', 'readiness', 'join', 'manual-save', 'mission'], 'roster'),
    group(['comms', 'activity', 'journal', 'session-history'], 'comms'),
  ] },
  floats: [], closed: [], selected: 'roster' });
const v3Default = () => ({ version: 3,
  root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
    { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      group(['roster', 'readiness', 'join', 'manual-save', 'mission',
        'attention', 'workload', 'widgets'], 'roster'),
      group(['map'], 'map'),
    ] },
    group(['comms', 'activity', 'journal', 'session-history', 'health'], 'comms'),
  ] },
  floats: [], closed: [], selected: 'roster' });
export function defaultLiveLayout() {
  return { version: LIVE_LAYOUT_VERSION,
    root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
      { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
        group(['roster', 'readiness', 'join', 'manual-save', 'mission',
          'attention', 'workload', 'widgets', 'station'], 'roster'),
        group(['map', 'station-console'], 'map'),
      ] },
      group(['comms', 'activity', 'journal', 'session-history', 'health'], 'comms'),
    ] },
    floats: [], closed: [], selected: 'roster' };
}
const base = createDockLayoutModel({
  version: LIVE_LAYOUT_VERSION, panels: LIVE_PANEL_REGISTRY, defaultLayout: defaultLiveLayout,
  pinned: LIVE_PINNED_PANELS, compatibleVersions: [LIVE_LAYOUT_VERSION],
});
const v1 = createDockLayoutModel({ version: 1, panels: V1_PANELS, defaultLayout: v1Default,
  compatibleVersions: [1] });
const v2 = createDockLayoutModel({ version: 2, panels: V2_PANELS, defaultLayout: v2Default,
  compatibleVersions: [2] });
// No `pinned` here on purpose: a generation model only sanitizes a stored tree.
// The pinned repair runs once, at the end of migration, so the browser and the
// native profile sanitizer place a repaired panel in the same tab position.
const v3 = createDockLayoutModel({ version: 3, panels: V3_PANELS, defaultLayout: v3Default,
  compatibleVersions: [3] });
const migrate = createDockLayoutMigration({
  version: LIVE_LAYOUT_VERSION, current: base,
  generations: [
    { version: 1, model: v1, added: [] },
    { version: 2, model: v2, added: ADDED_IN_V2 },
    { version: 3, model: v3, added: ADDED_IN_V3 },
    { version: LIVE_LAYOUT_VERSION, model: base, added: ADDED_IN_V4 },
  ],
});
export const liveLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeLiveLayout = migrate;
