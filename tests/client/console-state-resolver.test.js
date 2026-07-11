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
