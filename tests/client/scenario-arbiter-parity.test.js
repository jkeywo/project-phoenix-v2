/**
 * tests/client/scenario-arbiter-parity.test.js — the JavaScript half of the
 * shared arbiter parity table.
 *
 * `tests/fixtures/scenario-arbiter-parity.json` is a case table consumed by two
 * test suites: this one, driving `gui/scenario-arbiter.js`, and the
 * `scenario_arbiter_parity_*` tests in `src/lobby/scenario_arbiter.rs`, driving
 * the Rust transcription of the same rules. Neither suite owns the cases;
 * neither may skip one. New behaviour goes in the JSON, and a case only one side
 * can satisfy is a bug in that side.
 *
 * `tests/client/scenario-arbiter.test.js` still stands beside this: it is the
 * JS's own unit suite, covering shapes (`findScenario(null, …)`,
 * `normalizeSelection(undefined)`) that have no Rust counterpart.
 */
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  selectScenario,
  selectPlayerShip,
  isComplete,
  worldPathFor,
  curatedShipsFor,
} from '../../gui/scenario-arbiter.js';

const FIXTURE = JSON.parse(
  readFileSync(new URL('../fixtures/scenario-arbiter-parity.json', import.meta.url), 'utf8'),
);

const CATALOG = FIXTURE.catalog;

/** The two entry points, keyed by the table's `call` field. */
const CALLS = {
  select_scenario: selectScenario,
  select_player_ship: selectPlayerShip,
};

describe('scenario-arbiter parity table — outcomes', () => {
  it('the table is non-empty (a silently unread fixture proves nothing)', () => {
    expect(FIXTURE.cases.length).toBeGreaterThan(0);
    expect(FIXTURE.derived.length).toBeGreaterThan(0);
  });

  for (const testCase of FIXTURE.cases) {
    it(`${testCase.call}: ${testCase.name}`, () => {
      const call = CALLS[testCase.call];
      expect(call, `unknown call "${testCase.call}"`).toBeTypeOf('function');
      const result = call(testCase.selection, CATALOG, testCase.argument);
      expect(result.outcome).toBe(testCase.outcome);
      expect(result.selection).toEqual(testCase.selection_after);
    });
  }
});

describe('scenario-arbiter parity table — derived answers', () => {
  for (const row of FIXTURE.derived) {
    it(row.name, () => {
      expect(isComplete(row.selection)).toBe(row.is_complete);
      expect(worldPathFor(CATALOG, row.selection)).toBe(row.world_path);
      expect(curatedShipsFor(CATALOG, row.selection)).toEqual(row.curated_ships);
    });
  }
});
