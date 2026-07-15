// @vitest-environment jsdom
//
// Regression test for the 'engineering' console routing in
// window.buildConsoleStateInner (gui/console-state.js). Ship TOMLs give the
// shields system id = "shields-system" (kind = "shields"); the resolver must
// check for that exact id, not the bare kind name, or the Destroyer's merged
// Engineering console silently loses its shields sub-object and the shield
// panel renders empty.
import { describe, it, expect } from 'vitest';
import '../../gui/console-state.js';
import { aggregateStationHull } from '../../gui/console-state.js';

describe('buildConsoleStateInner engineering routing', () => {
  it('routes to the Destroyer builder (shields included) when station_systems lists "shields-system"', () => {
    const state = {
      stationSystems: { engineering: ['shields-system', 'power-reactor', 'power-battery', 'repair'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('engineering', state));
    expect(s).toHaveProperty('shields');
    expect(s).toHaveProperty('power');
    expect(s).toHaveProperty('repair');
  });

  it('routes to the plain Cruiser-style builder (no shields) when engineering has no shields system', () => {
    const state = {
      stationSystems: { engineering: ['power-reactor', 'power-battery', 'repair'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('engineering', state));
    expect(s).not.toHaveProperty('shields');
    expect(s).toHaveProperty('power');
    expect(s).toHaveProperty('repair');
  });
});

describe('buildConsoleStateInner pilot routing', () => {
  it("routes the Courier's 'pilot' station to the combined single-console builder", () => {
    const s = JSON.parse(window.buildConsoleStateInner('pilot', {}));
    for (const key of ['weapons', 'sensors', 'navigation', 'comms', 'captain', 'helm']) {
      expect(s).toHaveProperty(key);
    }
  });

  // Every nested sub-object computes an own_hull for its own hardcoded station
  // id ('tactical', 'sensors', ...), which is meaningless on a hull with one
  // station. The top-level own_hull — the only one pilot.html reads — must be
  // the pilot's, or the footer damage bar shows the wrong systems.
  it("top-level own_hull aggregates the pilot station, not tactical", () => {
    const state = {
      stationSystems: { pilot: ['blaster-fore'] },
      consoleHull: [
        { system_id: 'blaster-fore', display_name: 'Blaster', hp: 6, max_hp: 12, tier: 'Damaged' },
      ],
    };
    const s = JSON.parse(window.buildConsoleState('pilot', state));
    // Round-trip the expectation too: withStationDamage stringifies, which
    // drops undefined-valued keys.
    expect(s.own_hull).toEqual(
      JSON.parse(JSON.stringify(
        aggregateStationHull('pilot', state.consoleHull, state.stationSystems)
      ))
    );
    expect(s.own_hull).not.toBeNull();
    expect(s.own_hull.entries.map((e) => e.system_id)).toEqual(['blaster-fore']);
  });
});
