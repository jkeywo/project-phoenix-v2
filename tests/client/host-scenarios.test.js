// tests/client/host-scenarios.test.js — issue #1230: server.html's
// renderScenarioLockState()/renderModPackList() DOM writers are closed over
// host state, so there was no importable module to call. This suite exercises
// the pure view-models lifted out of them (gui/host-scenarios.js) directly:
// scenarioCatalogView (which picker stage to show, or the single-ship
// auto-resolve) and modPackListView (applied-pack rows + reorder eligibility
// + conflict rows). No DOM, no wasmBindings.

import { describe, it, expect } from 'vitest';
import { scenarioCatalogView, modPackListView } from '../../gui/host-scenarios.js';

// Mirrors the #754 pre-load catalog shape (wasm_get_scenario_catalog()) —
// same fixture shape as tests/client/scenario-arbiter.test.js.
const CATALOG = [
  {
    id: 'default',
    world: 'assets/worlds/default.toml',
    label: 'Starbase Alpha',
    ships: [
      { template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser' },
      { template_path: 'assets/entities/alliance_destroyer.toml', label: 'Destroyer' },
    ],
  },
  {
    id: 'combat_test',
    world: 'assets/worlds/combat_test.toml',
    label: 'Combat Test',
    ships: [{ template_path: 'assets/entities/alliance_battleship.toml', label: 'Battleship' }],
  },
];

const EMPTY_SEL = { scenario_id: null, template_path: null };

describe('scenarioCatalogView', () => {
  it('is locked when the world load has already started, regardless of selection', () => {
    expect(scenarioCatalogView(CATALOG, EMPTY_SEL, true)).toEqual({ stage: 'locked' });
    const complete = { scenario_id: 'default', template_path: 'assets/entities/alliance_cruiser.toml' };
    expect(scenarioCatalogView(CATALOG, complete, true)).toEqual({ stage: 'locked' });
  });

  it('is locked once both scenario and ship are chosen, even if `locked` is false', () => {
    const complete = { scenario_id: 'default', template_path: 'assets/entities/alliance_cruiser.toml' };
    expect(scenarioCatalogView(CATALOG, complete, false)).toEqual({ stage: 'locked' });
  });

  it('lists every catalog scenario when nothing is selected yet', () => {
    const vm = scenarioCatalogView(CATALOG, EMPTY_SEL, false);
    expect(vm.stage).toBe('scenario-list');
    expect(vm.labelId).toBe('server.select_world');
    expect(vm.entries.map((e) => [e.scenarioId, e.world, e.label])).toEqual([
      ['default', 'assets/worlds/default.toml', 'Starbase Alpha'],
      ['combat_test', 'assets/worlds/combat_test.toml', 'Combat Test'],
    ]);
    // Nothing is chosen yet, so no row is.
    expect(vm.entries.every((e) => e.selected === false)).toBe(true);
  });

  it('reports an empty catalog distinctly from a populated one', () => {
    const vm = scenarioCatalogView([], EMPTY_SEL, false);
    expect(vm).toEqual({ stage: 'scenario-empty', labelId: 'server.select_world' });
  });

  it('offers the ship picker once a multi-ship scenario is locked', () => {
    const sel = { scenario_id: 'default', template_path: null };
    const vm = scenarioCatalogView(CATALOG, sel, false);
    expect(vm.stage).toBe('ship-picker');
    expect(vm.labelId).toBe('server.select_ship');
    expect(vm.ships).toEqual(CATALOG[0].ships);
  });

  it('auto-resolves a single-ship scenario instead of showing a picker (issue #917)', () => {
    const sel = { scenario_id: 'combat_test', template_path: null };
    const vm = scenarioCatalogView(CATALOG, sel, false);
    expect(vm).toEqual({ stage: 'ship-auto', templatePath: 'assets/entities/alliance_battleship.toml' });
  });

  it('treats a locked scenario id absent from the catalog as offering no ships', () => {
    const sel = { scenario_id: 'unknown', template_path: null };
    const vm = scenarioCatalogView(CATALOG, sel, false);
    expect(vm.stage).toBe('ship-picker');
    expect(vm.labelId).toBe('server.select_ship');
    expect(vm.ships).toEqual([]);
    // The World rows still come, and none of them is the one that was locked.
    expect(vm.entries.map((e) => e.scenarioId)).toEqual(['default', 'combat_test']);
    expect(vm.entries.every((e) => e.selected === false)).toBe(true);
  });

  it('keeps the World rows through the hull stage, with the chosen one marked', () => {
    // Issue #1362. The hull stage used to carry the hulls and nothing else,
    // and the renderer put them where the World list had been — so an
    // operator who mis-clicked had no list to read and no row to go back to.
    const sel = { scenario_id: 'default', template_path: null };
    const vm = scenarioCatalogView(CATALOG, sel, false);
    expect(vm.scenarioId).toBe('default');
    expect(vm.worldLabelId).toBe('server.select_world');
    expect(vm.entries.map((e) => [e.scenarioId, e.selected])).toEqual([
      ['default', true],
      ['combat_test', false],
    ]);
  });

  it('says on every World row how many hulls it offers, before it is chosen', () => {
    // The row a click will ask a second question about, and the row that goes
    // straight through, were the same button until issue #1362. The count is
    // an {id, params} pair with a plural id, not prose: the count decides
    // which id, the string table decides the words.
    const vm = scenarioCatalogView(CATALOG, EMPTY_SEL, false);
    expect(vm.entries.map((e) => [e.hullCount, e.hasChoice, e.hullCountLabel])).toEqual([
      [2, true, { id: 'server.hulls_offered.other', params: { n: 2 } }],
      [1, false, { id: 'server.hulls_offered.one', params: { n: 1 } }],
    ]);
  });

  it('draws no count at all for a World that publishes no curated hull list', () => {
    // Not "0 hulls": an empty `ships` is what `curatedShipsFor` reads as
    // UNRESTRICTED — every hull the world itself holds — so a zero would be
    // the one reading an operator must not take from the row. Silence is the
    // honest rendering of "not known from here".
    const open = [{ id: 'open', world: 'assets/worlds/open.toml', label: 'Open' }];
    const vm = scenarioCatalogView(open, EMPTY_SEL, false);
    expect(vm.entries[0].hullCount).toBe(0);
    expect(vm.entries[0].hullCountLabel).toBe(null);
    expect(vm.entries[0].hasChoice).toBe(false);
  });

  it('normalizes a null/undefined preSelection to the empty shape', () => {
    expect(scenarioCatalogView(CATALOG, null, false).stage).toBe('scenario-list');
    expect(scenarioCatalogView(CATALOG, undefined, false).stage).toBe('scenario-list');
  });
});

describe('modPackListView', () => {
  it('is not visible when there are no applied packs', () => {
    expect(modPackListView({ packs: [], conflicts: [] })).toEqual({
      visible: false,
      packs: [],
      conflicts: [],
    });
    expect(modPackListView(null)).toEqual({ visible: false, packs: [], conflicts: [] });
    expect(modPackListView(undefined)).toEqual({ visible: false, packs: [], conflicts: [] });
  });

  it('marks only the first row as unable to move up and only the last as unable to move down', () => {
    const report = {
      packs: [
        { id: 'a', name: 'Pack A', version: '1.0.0', file_count: 3 },
        { id: 'b', name: 'Pack B', version: '2.0.0', file_count: 5 },
        { id: 'c', name: 'Pack C', version: '3.0.0', file_count: 1 },
      ],
    };
    const vm = modPackListView(report);
    expect(vm.visible).toBe(true);
    expect(vm.packs).toEqual([
      { id: 'a', name: 'Pack A', version: '1.0.0', fileCount: 3, canMoveUp: false, canMoveDown: true },
      { id: 'b', name: 'Pack B', version: '2.0.0', fileCount: 5, canMoveUp: true, canMoveDown: true },
      { id: 'c', name: 'Pack C', version: '3.0.0', fileCount: 1, canMoveUp: true, canMoveDown: false },
    ]);
  });

  it('falls back to id/empty/zero for a pack missing name, version, or file_count', () => {
    const vm = modPackListView({ packs: [{ id: 'bare' }] });
    expect(vm.packs[0]).toEqual({
      id: 'bare',
      name: 'bare',
      version: '',
      fileCount: 0,
      canMoveUp: false,
      canMoveDown: false,
    });
  });

  it('carries conflicts through with losers flattened to an array', () => {
    const report = {
      packs: [{ id: 'a', name: 'A', version: '1', file_count: 1 }],
      conflicts: [{ path: 'assets/worlds/shared.toml', winner: 'b', losers: ['a', 'c'] }],
    };
    const vm = modPackListView(report);
    expect(vm.conflicts).toEqual([
      { path: 'assets/worlds/shared.toml', winner: 'b', losers: ['a', 'c'] },
    ]);
  });

  it('defaults a conflict with no losers field to an empty array', () => {
    const report = {
      packs: [{ id: 'a', name: 'A', version: '1', file_count: 1 }],
      conflicts: [{ path: 'x', winner: 'a' }],
    };
    expect(modPackListView(report).conflicts).toEqual([{ path: 'x', winner: 'a', losers: [] }]);
  });
});
