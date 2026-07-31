import { describe, it, expect } from 'vitest';
import {
  selectScenario,
  selectPlayerShip,
  isComplete,
  worldPathFor,
  findScenario,
  normalizeSelection,
  curatedShipsFor,
} from '../../gui/scenario-arbiter.js';

// Mirrors the #754 pre-load catalog shape delivered by wasm_get_scenario_catalog.
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

const EMPTY = { scenario_id: null, template_path: null };

describe('scenario-arbiter — scenario first-valid-wins', () => {
  it('accepts and locks the first valid scenario', () => {
    const r = selectScenario(EMPTY, CATALOG, 'default');
    expect(r.outcome).toBe('accepted');
    expect(r.selection).toEqual({ scenario_id: 'default', template_path: null });
  });

  it('rejects an unknown scenario id', () => {
    const r = selectScenario(EMPTY, CATALOG, 'does_not_exist');
    expect(r.outcome).toBe('rejected');
    expect(r.selection).toEqual(EMPTY);
  });

  it('ignores a second scenario once one is locked (first wins)', () => {
    const locked = { scenario_id: 'default', template_path: null };
    const r = selectScenario(locked, CATALOG, 'combat_test');
    expect(r.outcome).toBe('ignored');
    expect(r.selection.scenario_id).toBe('default');
  });
});

describe('scenario-arbiter — ship scoped to the locked scenario', () => {
  it('accepts a ship offered by the locked scenario', () => {
    const locked = { scenario_id: 'default', template_path: null };
    const r = selectPlayerShip(locked, CATALOG, 'assets/entities/alliance_destroyer.toml');
    expect(r.outcome).toBe('accepted');
    expect(r.selection).toEqual({
      scenario_id: 'default',
      template_path: 'assets/entities/alliance_destroyer.toml',
    });
  });

  it('rejects a ship not offered by the locked scenario (scenario scoping, #754 AC4)', () => {
    const locked = { scenario_id: 'default', template_path: null };
    // battleship belongs to combat_test, not default.
    const r = selectPlayerShip(locked, CATALOG, 'assets/entities/alliance_battleship.toml');
    expect(r.outcome).toBe('rejected');
    expect(r.selection.template_path).toBeNull();
  });

  it('rejects a ship before any scenario is locked', () => {
    const r = selectPlayerShip(EMPTY, CATALOG, 'assets/entities/alliance_cruiser.toml');
    expect(r.outcome).toBe('rejected');
  });

  it('ignores a second ship once one is locked (first wins)', () => {
    const locked = {
      scenario_id: 'default',
      template_path: 'assets/entities/alliance_cruiser.toml',
    };
    const r = selectPlayerShip(locked, CATALOG, 'assets/entities/alliance_destroyer.toml');
    expect(r.outcome).toBe('ignored');
    expect(r.selection.template_path).toBe('assets/entities/alliance_cruiser.toml');
  });
});

describe('scenario-arbiter — completion + world resolution', () => {
  it('is complete only when both scenario and ship are locked', () => {
    expect(isComplete(EMPTY)).toBe(false);
    expect(isComplete({ scenario_id: 'default', template_path: null })).toBe(false);
    expect(
      isComplete({ scenario_id: 'default', template_path: 'assets/entities/alliance_cruiser.toml' }),
    ).toBe(true);
  });

  it('resolves the world path for the locked scenario and drives the full flow', () => {
    // A full valid pair from both participants: server picks scenario, phone picks ship.
    let sel = EMPTY;
    sel = selectScenario(sel, CATALOG, 'combat_test').selection;
    sel = selectPlayerShip(sel, CATALOG, 'assets/entities/alliance_battleship.toml').selection;
    expect(isComplete(sel)).toBe(true);
    expect(worldPathFor(CATALOG, sel)).toBe('assets/worlds/combat_test.toml');
  });

  it('worldPathFor returns null for an unresolved selection', () => {
    expect(worldPathFor(CATALOG, EMPTY)).toBeNull();
  });
});

describe('scenario-arbiter — curatedShipsFor (issue #917 preload allowlist)', () => {
  it('returns the locked scenario entry\'s ship template paths', () => {
    const locked = { scenario_id: 'combat_test', template_path: null };
    expect(curatedShipsFor(CATALOG, locked)).toEqual([
      'assets/entities/alliance_battleship.toml',
    ]);
  });

  it('returns every ship the entry lists when the scenario is uncurated', () => {
    const locked = { scenario_id: 'default', template_path: null };
    expect(curatedShipsFor(CATALOG, locked)).toEqual([
      'assets/entities/alliance_cruiser.toml',
      'assets/entities/alliance_destroyer.toml',
    ]);
  });

  it('returns an empty array (unrestricted) for an unresolved selection', () => {
    expect(curatedShipsFor(CATALOG, EMPTY)).toEqual([]);
  });

  it('returns an empty array for an unknown scenario id', () => {
    expect(curatedShipsFor(CATALOG, { scenario_id: 'nope', template_path: null })).toEqual([]);
  });
});

describe('scenario-arbiter — helpers', () => {
  it('findScenario returns the entry or null', () => {
    expect(findScenario(CATALOG, 'default').world).toBe('assets/worlds/default.toml');
    expect(findScenario(CATALOG, 'nope')).toBeNull();
    expect(findScenario(null, 'default')).toBeNull();
  });

  it('normalizeSelection coerces missing fields to null', () => {
    expect(normalizeSelection(undefined)).toEqual(EMPTY);
    expect(normalizeSelection({ scenario_id: 'x' })).toEqual({ scenario_id: 'x', template_path: null });
  });
});
