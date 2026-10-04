import { PANEL_KIND } from './dock-layout-model.js';
import { createDockLayoutHistory } from './dock-layout-history.js';

export const LIVE_LAYOUT_VERSION = 19;
// Registry order is independent of default groups and migration placement.
// No target means a draft is registered but never opened by migration.
const records = [
  ['roster', 1, 'workflow'], ['readiness', 1, 'workflow'], ['join', 1, 'workflow'],
  ['manual-save', 1, 'workflow'],
  ['mission', 2, 'workflow', 'roster'],
  ['comms', 2, 'records', 'roster', 'bottom'],
  ['activity', 2, 'records', 'comms'], ['journal', 2, 'records', 'comms'],
  ['session-history', 2, 'records', 'comms'],
  ['map', 3, 'map', 'roster', 'right'],
  ['attention', 3, 'workflow', 'roster'], ['workload', 3, 'workflow', 'roster'],
  ['widgets', 3, 'workflow', 'roster'], ['health', 3, 'records', 'comms'],
  ['station', 4, 'workflow', 'roster'], ['station-console', 4, 'map', 'map'],
  ['presentation', 5, 'utilities', 'mission', 'bottom'],
  ['audition', 5, 'utilities', 'presentation'], ['source-link', 5, 'utilities', 'presentation'],
  ['spawn', 6], ['inspector', 7, 'inspector', 'map', 'right'],
  ['checkpoint', 8, 'records', 'journal'], ['restore', 8],
  ['contact', 9, 'inspector', 'inspector'], ['npc', 9, 'inspector', 'inspector'],
  ['misclassify', 9], ['report-policy', 9], ['ghost', 9],
  ['system', 10, 'inspector', 'inspector'], ['effect', 10],
  ['despawn', 11, 'inspector', 'inspector'], ['faction', 11, 'inspector', 'inspector'],
  ['objective', 12, 'workflow', 'mission'], ['entity-fields', 13, 'inspector', 'inspector'],
  ['world-fields', 15, 'workflow', 'mission'],
  ['hull-fields', 16, 'inspector', 'entity-fields'],
  ['region-fields', 17, 'inspector', 'entity-fields'],
  ['presentation-fields', 18, 'inspector', 'region-fields'],
].map(([id, since, home, target, placement]) => ({
  id, since, home,
  kind: ['map', 'station-console', 'inspector'].includes(id) ? PANEL_KIND.DOCUMENT : PANEL_KIND.TOOL,
  // Ghost left the UI in v14, but persisted vocabulary accepted it through v17.
  ...(id === 'ghost' ? { retired: 18 } : {}),
  ...(target ? { migration: { target, placement } } : {}),
}));

/** A floating draft is not restored; a docked draft returns empty. */
export const LIVE_TEMPORARY_PANELS = Object.freeze([
  'spawn', 'restore', 'misclassify', 'report-policy', 'effect',
  'manual-save', 'contact', 'npc', 'system', 'despawn', 'faction',
  'entity-fields', 'world-fields', 'hull-fields', 'region-fields', 'presentation-fields',
]);
const TEMPORARY_UNTIL_V13 = Object.freeze([...LIVE_TEMPORARY_PANELS.slice(0, 4), 'ghost',
  ...LIVE_TEMPORARY_PANELS.slice(4)]);
// Critical warnings live in the header, independently of dock placement.
export const LIVE_PINNED_PANELS = Object.freeze([]);
const group = (tabs, active = tabs[0]) => ({ type: 'tabs', tabs, active });
const split = (axis, children, sizes = [1, 1]) => ({ type: 'split', axis, sizes, children });

// These fallback membership exceptions are persisted history, not new-panel
// introductions: v5–13 defaults leaked world-fields, and v9–13 defaults leaked
// the later inspector fields. Sanitizing a valid stored tree still uses since.
function defaultMember(panel, version) {
  if (version >= 5 && version <= 13 && panel.id === 'world-fields') return true;
  if (version >= 9 && version <= 13
    && ['hull-fields', 'region-fields', 'presentation-fields'].includes(panel.id)) return true;
  return panel.since <= version;
}
function historicalDefault(version) {
  if (version >= 18) return { ...defaultLiveLayout(), version };
  const home = name => group(records.filter(panel => panel.home === name && defaultMember(panel, version))
    .map(panel => panel.id));
  let root = home('workflow');
  if (version >= 2) {
    let documents = root;
    if (version >= 3) {
      let workflow = home('workflow');
      if (version >= 5) workflow = split('vertical', [workflow, home('utilities')]);
      let map = home('map');
      if (version >= 7) map = split('horizontal', [map, home('inspector')]);
      documents = split('horizontal', [workflow, map]);
    }
    root = split('vertical', [documents, home('records')]);
  }
  return { version, root, floats: [],
    closed: records.filter(panel => !panel.home && panel.since <= version
      && (panel.id !== 'ghost' || version < 14)).map(panel => panel.id),
    selected: 'roster' };
}

export function defaultLiveLayout() {
  return { version: LIVE_LAYOUT_VERSION,
    root: split('vertical', [
      split('horizontal', [group(['roster']), group(['map']), group(['inspector'])], [22, 52, 26]),
      group(['activity']),
    ], [4, 1]),
    floats: [], closed: LIVE_PANELS.filter(id => !['roster', 'map', 'inspector', 'activity'].includes(id)),
    selected: 'roster' };
}
const { registry, current: base, migrate } = createDockLayoutHistory({
  version: LIVE_LAYOUT_VERSION, panels: records, defaultLayout: historicalDefault,
  policy: version => version === LIVE_LAYOUT_VERSION
    ? { temporary: LIVE_TEMPORARY_PANELS, pinned: LIVE_PINNED_PANELS }
    : version >= 6 && version <= 13 ? { temporary: TEMPORARY_UNTIL_V13 }
      : version >= 14 && version <= 17 ? { temporary: LIVE_TEMPORARY_PANELS }
        // Version 18 intentionally had no temporary-panel policy.
        : {},
});
export const LIVE_PANEL_REGISTRY = registry;
export const LIVE_PANELS = base.panels;
export function normalizeLiveLayout(value, bounds) {
  return value?.version === LIVE_LAYOUT_VERSION ? base.normalize(value, bounds) : base.defaultLayout();
}
export function restorePreviousLiveLayout(value) {
  const aliases = { join: 'readiness', health: 'readiness', objective: 'mission', journal: 'activity',
    'session-history': 'activity', station: 'station-console' };
  const rewrite = node => !node ? null : node.type === 'tabs'
    ? { ...node, tabs: node.tabs.map(id => aliases[id] || id), active: aliases[node.active] || node.active }
    : { ...node, children: node.children.map(rewrite) };
  const old = migrate(value);
  return base.normalize({ ...old, version: LIVE_LAYOUT_VERSION, root: rewrite(old.root),
    floats: old.floats.map(row => ({ ...row, panel: aliases[row.panel] || row.panel })),
    selected: aliases[old.selected] || old.selected });
}
export const liveLayoutModel = Object.freeze({ ...base, normalize: normalizeLiveLayout });
