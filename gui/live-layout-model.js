import { createDockLayoutModel, PANEL_KIND } from './dock-layout-model.js';
import { createDockLayoutMigration } from './dock-layout-migration.js';

export const LIVE_LAYOUT_VERSION = 14;
const tool = id => Object.freeze({ id, kind: PANEL_KIND.TOOL });
const documentPanel = id => Object.freeze({ id, kind: PANEL_KIND.DOCUMENT });
export const LIVE_PANEL_REGISTRY = Object.freeze([
  tool('roster'), tool('readiness'), tool('join'), tool('manual-save'),
  tool('mission'), tool('comms'), tool('activity'), tool('journal'), tool('session-history'),
  documentPanel('map'), tool('attention'), tool('workload'), tool('widgets'), tool('health'),
  tool('station'), documentPanel('station-console'),
  tool('presentation'), tool('audition'), tool('source-link'),
  tool('spawn'), documentPanel('inspector'),
  tool('checkpoint'), tool('restore'),
  tool('contact'), tool('npc'),
  tool('misclassify'), tool('report-policy'),
  tool('system'), tool('effect'),
  tool('despawn'), tool('faction'),
  tool('objective'),
  tool('entity-fields'),
]);
export const LIVE_PANELS = Object.freeze(LIVE_PANEL_REGISTRY.map(panel => panel.id));
const V1_PANELS = Object.freeze(['roster', 'readiness', 'join', 'manual-save']);
const V2_PANELS = Object.freeze([...V1_PANELS,
  'mission', 'comms', 'activity', 'journal', 'session-history']);
const V3_PANELS = Object.freeze([...V2_PANELS, 'map', 'attention', 'workload', 'widgets', 'health']);
const V4_PANELS = Object.freeze([...V3_PANELS, 'station', 'station-console']);
const V5_PANELS = Object.freeze([...V4_PANELS, 'presentation', 'audition', 'source-link']);
const V6_PANELS = Object.freeze([...V5_PANELS, 'spawn']);
const V7_PANELS = Object.freeze([...V6_PANELS, 'inspector']);
const V8_PANELS = Object.freeze([...V7_PANELS, 'checkpoint', 'restore']);
const V9_PANELS = Object.freeze([...V8_PANELS,
  'contact', 'npc', 'misclassify', 'report-policy', 'ghost']);
const V10_PANELS = Object.freeze([...V9_PANELS, 'system', 'effect']);
const V11_PANELS = Object.freeze([...V10_PANELS, 'despawn', 'faction']);
const V12_PANELS = Object.freeze([...V11_PANELS, 'objective']);
const V13_PANELS = Object.freeze([...V12_PANELS, 'entity-fields']);
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
/** Complex actions the operator opens, fills in and finishes. They are absent
 * from the default arrangement and open as a floating draft; a DOCKED one is a
 * tool kept to hand and comes back empty, and a floating one is not restored at
 * all. Spawn is the first (issue #1506); the later complex actions join it. */
export const LIVE_TEMPORARY_PANELS = Object.freeze([
  'spawn', 'restore',
  // Each of these combines an observer and a target with a classification or a
  // delay/quantisation/privacy policy — several choices before anything can be
  // sent, which is what makes it a draft rather than a verb (issue #1510). The
  // ghost draft that stood beside them was retired in version 14: placing a
  // ghost is a Spawn OUTCOME now, the same palette and the same chart gesture,
  // reported to a ship instead of spawned into the world.
  'misclassify', 'report-policy',
  // Direct damage and repair combine an effect kind, an amount, a scope over
  // the hull, a Station or one System, and a clamp/lethality preview — several
  // choices about one press, which is what makes it a draft (issue #1511).
  'effect',
]);
/** The draft vocabulary every version up to 13 stored, which still carried the
 * ghost draft. A stored tree is sanitized against ITS version's drafts — a
 * floating ghost draft in a version-13 profile was a draft then, and is not
 * restored — before the current registry drops the panel altogether. */
const TEMPORARY_UNTIL_V13 = Object.freeze([...LIVE_TEMPORARY_PANELS.slice(0, 4), 'ghost',
  ...LIVE_TEMPORARY_PANELS.slice(4)]);

/** The attention region renders connection and recovery banners verbatim and the
 * health panel is the readable table behind them. Neither may be hidden by a
 * role preset (they are absent from GM_ROLE_PRESET_PANEL_IDS) and neither may be
 * closed here either: a Game Master must not be able to hide a failure from
 * themselves, whichever mechanism does the hiding. */
export const LIVE_PINNED_PANELS = Object.freeze(['attention', 'health']);
/** Panels registered after version 4. The operator's own utilities — typed
 * presentation control, private sound audition and the one-way Workshop source
 * handoff — open a group of their own under the workflow panels: each is a
 * local instrument rather than a record or a projection. */
const ADDED_IN_V5 = Object.freeze([
  ['presentation', 'mission', 'bottom'],
  ['audition', 'presentation', 'tab'],
  ['source-link', 'presentation', 'tab'],
]);
/** Panels registered after version 6. The entity inspector is the desk's other
 * reading surface, and it gets a COLUMN rather than a tab beside the map: it
 * holds every selected-entity control — systems, contacts, doctrine,
 * objectives, direct effect, despawn, factions — and a desk whose actions start
 * out behind another panel's tab is a desk that starts out with its actions
 * hidden. That is the right-hand column the screen always had. */
const ADDED_IN_V7 = Object.freeze([['inspector', 'map', 'right']]);
/** Panels registered after version 7. Checkpoint browsing is a RECORD — bounded,
 * current, read beside the journal and the session history — while restore is a
 * complex action: it combines a selection, a preflight, a consequence preview
 * and a confirmation, so it is a draft like Spawn. */
const ADDED_IN_V8 = Object.freeze([['checkpoint', 'journal', 'tab']]);
/** Panels registered after version 8. Contact information and NPC doctrine are
 * ordinary tools about the selected entity, so they join the inspector. */
const ADDED_IN_V9 = Object.freeze([
  ['contact', 'inspector', 'tab'],
  ['npc', 'inspector', 'tab'],
]);
/** Panels registered after version 9. Disabling and restoring one authored
 * System is a target-relative choice and a verb, so it is an ordinary tool
 * beside the selection it reads. */
const ADDED_IN_V10 = Object.freeze([['system', 'inspector', 'tab']]);
/** Panels registered after version 10. Removing one selected entity, and
 * setting an ordered faction pair's absolute hostility, are both one choice and
 * a verb — neither composes a draft — so both are ordinary tools beside the
 * selection and the projection they read (issue #1512). */
const ADDED_IN_V11 = Object.freeze([
  ['despawn', 'inspector', 'tab'],
  ['faction', 'inspector', 'tab'],
]);
/** Panels registered after version 11. Activating, completing and failing an
 * authored Objective is the mission workflow's own vocabulary — the authored
 * target and its recipients already define the operation — so it joins the
 * mission events it belongs to rather than the selected-entity column. It
 * narrows to the selected ship where there is one, which is why the inspector's
 * shortcut still points at it; pointing is not a second copy (issue #1513). */
const ADDED_IN_V12 = Object.freeze([['objective', 'mission', 'tab']]);
/** Panels registered after version 12. The entities/AI Live Inspector reads the
 * same selection the entity inspector does, so it joins that column as a tab
 * rather than opening a surface of its own (issue #1489). */
const ADDED_IN_V13 = Object.freeze([['entity-fields', 'inspector', 'tab']]);
/** Nothing. Version 14 retires the ghost draft (see LIVE_TEMPORARY_PANELS). */
const ADDED_IN_V14 = Object.freeze([]);
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
const v4Default = () => ({ version: 4,
  root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
    { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      group(['roster', 'readiness', 'join', 'manual-save', 'mission',
        'attention', 'workload', 'widgets', 'station'], 'roster'),
      group(['map', 'station-console'], 'map'),
    ] },
    group(['comms', 'activity', 'journal', 'session-history', 'health'], 'comms'),
  ] },
  floats: [], closed: [], selected: 'roster' });
/** The arrangement every version from 5 on starts from. A temporary panel is
 * absent from it by definition: a draft nobody has opened is not a place. */
const liveArrangement = () => ({
  root: { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
    { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
      { type: 'split', axis: 'vertical', sizes: [1, 1], children: [
        group(['roster', 'readiness', 'join', 'manual-save', 'mission',
          'attention', 'workload', 'widgets', 'station', 'objective'], 'roster'),
        group(['presentation', 'audition', 'source-link'], 'presentation'),
      ] },
      { type: 'split', axis: 'horizontal', sizes: [1, 1], children: [
        group(['map', 'station-console'], 'map'),
        group(['inspector', 'contact', 'npc', 'system', 'despawn', 'faction', 'entity-fields'],
          'inspector'),
      ] },
    ] },
    group(['comms', 'activity', 'journal', 'session-history', 'health', 'checkpoint'], 'comms'),
  ] },
});
/** The arrangement before the entities/AI Live Inspector joined the inspector
 * column (version 13). */
const arrangementBeforeV13 = () => {
  const arrangement = liveArrangement();
  const documents = arrangement.root.children[0].children[1].children[1];
  documents.tabs = documents.tabs.filter(panel => panel !== 'entity-fields');
  return arrangement;
};
/** The arrangement before the authored Objective controls joined the mission
 * workflow (version 12). */
const arrangementBeforeV12 = () => {
  const arrangement = arrangementBeforeV13();
  const workflow = arrangement.root.children[0].children[0].children[0];
  workflow.tabs = workflow.tabs.filter(panel => panel !== 'objective');
  return arrangement;
};
/** The arrangement before the inspector took a column of its own (version 7). */
const arrangementBeforeV7 = () => {
  const arrangement = arrangementBeforeV8();
  const documents = arrangement.root.children[0];
  documents.children[1] = documents.children[1].children[0];
  return arrangement;
};
/** The arrangement before removal and faction hostility joined the inspector
 * (version 11). */
const arrangementBeforeV11 = () => {
  const arrangement = arrangementBeforeV12();
  const documents = arrangement.root.children[0].children[1].children[1];
  documents.tabs = documents.tabs.filter(panel => panel !== 'despawn' && panel !== 'faction');
  return arrangement;
};
/** The arrangement before System control joined the inspector (version 10). */
const arrangementBeforeV10 = () => {
  const arrangement = arrangementBeforeV11();
  const documents = arrangement.root.children[0].children[1].children[1];
  documents.tabs = documents.tabs.filter(panel => panel !== 'system');
  return arrangement;
};
/** The arrangement before the contact and doctrine tools joined the inspector
 * (version 9). */
const arrangementBeforeV9 = () => {
  const arrangement = arrangementBeforeV10();
  const documents = arrangement.root.children[0].children[1].children[1];
  documents.tabs = ['inspector'];
  return arrangement;
};
/** The arrangement before checkpoint browsing joined the records (version 8). */
const arrangementBeforeV8 = () => {
  const arrangement = arrangementBeforeV9();
  const records = arrangement.root.children[1];
  records.tabs = records.tabs.filter(panel => panel !== 'checkpoint');
  return arrangement;
};
const v5Default = () => ({ version: 5, ...arrangementBeforeV7(), floats: [], closed: [], selected: 'roster' });
const v6Default = () => ({ version: 6, ...arrangementBeforeV7(),
  floats: [], closed: ['spawn'], selected: 'roster' });
const v7Default = () => ({ version: 7, ...arrangementBeforeV8(),
  floats: [], closed: ['spawn'], selected: 'roster' });
const v8Default = () => ({ version: 8, ...arrangementBeforeV9(),
  floats: [], closed: ['spawn', 'restore'], selected: 'roster' });
const v9Default = () => ({ version: 9, ...arrangementBeforeV10(),
  floats: [],
  closed: ['spawn', 'restore', 'misclassify', 'report-policy', 'ghost'],
  selected: 'roster' });
const v10Default = () => ({ version: 10, ...arrangementBeforeV11(),
  floats: [],
  closed: ['spawn', 'restore', 'misclassify', 'report-policy', 'ghost', 'effect'],
  selected: 'roster' });
const v11Default = () => ({ version: 11, ...arrangementBeforeV12(),
  floats: [],
  closed: ['spawn', 'restore', 'misclassify', 'report-policy', 'ghost', 'effect'],
  selected: 'roster' });
const v12Default = () => ({ version: 12, ...arrangementBeforeV13(),
  floats: [],
  closed: ['spawn', 'restore', 'misclassify', 'report-policy', 'ghost', 'effect'],
  selected: 'roster' });
const v13Default = () => ({ version: 13, ...liveArrangement(),
  floats: [],
  closed: ['spawn', 'restore', 'misclassify', 'report-policy', 'ghost', 'effect'],
  selected: 'roster' });
export function defaultLiveLayout() {
  return { version: LIVE_LAYOUT_VERSION, ...liveArrangement(),
    floats: [],
    closed: ['spawn', 'restore', 'misclassify', 'report-policy', 'effect'],
    selected: 'roster' };
}
const base = createDockLayoutModel({
  version: LIVE_LAYOUT_VERSION, panels: LIVE_PANEL_REGISTRY, defaultLayout: defaultLiveLayout,
  pinned: LIVE_PINNED_PANELS, temporary: LIVE_TEMPORARY_PANELS,
  compatibleVersions: [LIVE_LAYOUT_VERSION],
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
const v4 = createDockLayoutModel({ version: 4, panels: V4_PANELS, defaultLayout: v4Default,
  compatibleVersions: [4] });
const v5 = createDockLayoutModel({ version: 5, panels: V5_PANELS, defaultLayout: v5Default,
  compatibleVersions: [5] });
const v6 = createDockLayoutModel({ version: 6, panels: V6_PANELS, defaultLayout: v6Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [6] });
const v7 = createDockLayoutModel({ version: 7, panels: V7_PANELS, defaultLayout: v7Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [7] });
const v8 = createDockLayoutModel({ version: 8, panels: V8_PANELS, defaultLayout: v8Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [8] });
const v9 = createDockLayoutModel({ version: 9, panels: V9_PANELS, defaultLayout: v9Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [9] });
const v10 = createDockLayoutModel({ version: 10, panels: V10_PANELS, defaultLayout: v10Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [10] });
const v11 = createDockLayoutModel({ version: 11, panels: V11_PANELS, defaultLayout: v11Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [11] });
const v12 = createDockLayoutModel({ version: 12, panels: V12_PANELS, defaultLayout: v12Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [12] });
const v13 = createDockLayoutModel({ version: 13, panels: V13_PANELS, defaultLayout: v13Default,
  temporary: TEMPORARY_UNTIL_V13, compatibleVersions: [13] });
const migrate = createDockLayoutMigration({
  version: LIVE_LAYOUT_VERSION, current: base,
  generations: [
    { version: 1, model: v1, added: [] },
    { version: 2, model: v2, added: ADDED_IN_V2 },
    { version: 3, model: v3, added: ADDED_IN_V3 },
    { version: 4, model: v4, added: ADDED_IN_V4 },
    { version: 5, model: v5, added: ADDED_IN_V5 },
    // A temporary panel is never PLACED by migration: it starts closed, which
    // is what "not open" means for a draft.
    { version: 6, model: v6, added: [] },
    { version: 7, model: v7, added: ADDED_IN_V7 },
    { version: 8, model: v8, added: ADDED_IN_V8 },
    { version: 9, model: v9, added: ADDED_IN_V9 },
    { version: 10, model: v10, added: ADDED_IN_V10 },
    { version: 11, model: v11, added: ADDED_IN_V11 },
    { version: 12, model: v12, added: ADDED_IN_V12 },
    { version: 13, model: v13, added: ADDED_IN_V13 },
    // Version 14 registers nothing: it RETIRES the ghost draft. A retired panel
    // needs no placement pass — the current registry does not know it, so the
    // final sanitize drops it from wherever a stored tree held it.
    { version: LIVE_LAYOUT_VERSION, model: base, added: ADDED_IN_V14 },
  ],
});
export const liveLayoutModel = Object.freeze({ ...base, normalize: migrate });
export const normalizeLiveLayout = migrate;
