// @vitest-environment jsdom
//
// Routing tests for window.buildConsoleStateInner (gui/console-state.js).
// Post issue #825 the routing rule is purely data-driven: a station whose
// TOML-owned fine systems span more than one console family (via
// consoleForSystemId) gets the generic system-id-keyed payload from
// buildSystemStationConsoleState; single-family stations keep their flat
// plain-builder payloads; unknown names (and known multi-family pages before
// Welcome delivers stationSystems) fall back to '{}'.
import { describe, it, expect } from 'vitest';
import '../../gui/console-state.js';
import { aggregateStationHull } from '../../gui/console-state.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { dirtyConsolesFor } from '../../gui/dirty-consoles.js';

describe('buildConsoleStateInner family-span routing', () => {
  it('routes the Destroyer engineering station (shields + power + repair) to the generic payload', () => {
    const state = {
      stationSystems: { engineering: ['shields-system', 'power-reactor', 'power-battery', 'repair'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('engineering', state));
    expect(s.station_id).toBe('engineering');
    expect(s.systems).toHaveProperty('shields-system');
    expect(s.systems).toHaveProperty('power-reactor');
    expect(s.systems).toHaveProperty('repair');
  });

  it('routes the Cruiser engineering station (power + repair, no shields) to the generic payload', () => {
    const state = {
      stationSystems: { engineering: ['power-reactor', 'power-battery', 'repair'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('engineering', state));
    expect(s.station_id).toBe('engineering');
    expect(s.systems).not.toHaveProperty('shields-system');
    expect(s.systems).toHaveProperty('power-reactor');
    expect(s.systems).toHaveProperty('repair');
  });

  it('routes the Cruiser science station (sensors + shields) to the generic payload', () => {
    const state = {
      stationSystems: { science: ['sensors', 'sensor-radar', 'shields-system'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('science', state));
    expect(s.station_id).toBe('science');
    expect(s.systems).toHaveProperty('sensors');
    expect(s.systems).toHaveProperty('shields-system');
  });

  it('routes the Cruiser comms station (navigation + comms) to the generic payload', () => {
    const state = {
      stationSystems: { comms: ['navigation', 'comms'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('comms', state));
    expect(s.station_id).toBe('comms');
    expect(s.systems).toHaveProperty('navigation');
    expect(s.systems).toHaveProperty('comms');
  });

  it('routes the Destroyer captain station (command + sensors) to the generic payload', () => {
    const state = {
      stationSystems: { captain: ['captain', 'red-alert', 'viewscreen', 'sensors', 'sensor-radar'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('captain', state));
    expect(s.station_id).toBe('captain');
    expect(s.systems).toHaveProperty('captain');
    expect(s.systems).toHaveProperty('sensors');
  });

  it('routes the Destroyer tactical station (weapons + navigation + comms) to the generic payload', () => {
    const state = {
      stationSystems: { tactical: ['tactical-radar', 'phaser-control', 'phaser-omni', 'navigation', 'comms'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('tactical', state));
    expect(s.station_id).toBe('tactical');
    expect(s.systems).toHaveProperty('tactical-radar');
    expect(s.systems).toHaveProperty('navigation');
    expect(s.systems).toHaveProperty('comms');
  });

  it('keeps the flat plain-builder payload for a single-family station (battleship tactical)', () => {
    const state = {
      stationSystems: { tactical: ['tactical-radar', 'phaser-control', 'phaser-fore', 'torpedo-magazine', 'blaster-heavy-fore'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('tactical', state));
    // Flat weapons shape — not the generic system-id-keyed wrapper.
    expect(s).not.toHaveProperty('systems');
    expect(s).toHaveProperty('banks');
    expect(s).toHaveProperty('tubes');
  });

  it('keeps the flat plain-builder payload for a single-family captain station', () => {
    const state = {
      stationSystems: { captain: ['captain', 'red-alert', 'viewscreen'] },
    };
    const s = JSON.parse(window.buildConsoleStateInner('captain', state));
    expect(s).not.toHaveProperty('systems');
    expect(s).toHaveProperty('red_alert');
  });

  it('selects the Command builder from projected metadata, not either id spelling', () => {
    const state = {
      stationSystems: { 'bridge-orders': ['orders-system'] },
      systemConsoleFamilies: { 'orders-system': 'command' },
      hasSystemConsoleFamilyProjection: true,
      blackboards: {
        command: {
          command_system_id: 'orders-system',
          directed_station: 'tactical',
          directed_station_name: 'Tactical',
          directed_station_ai: true,
          stances: [],
        },
      },
    };
    const s = JSON.parse(window.buildConsoleStateInner('bridge-orders', state));
    expect(s).not.toHaveProperty('systems');
    expect(s.command_system_id).toBe('orders-system');
    expect(s.directed_station).toBe('tactical');
  });

  it('does not extend the temporary inference fallback to an unmapped Command id', () => {
    const state = {
      stationSystems: { 'bridge-orders': ['orders-system'] },
      systemConsoleFamilies: {},
      hasSystemConsoleFamilyProjection: true,
    };
    expect(JSON.parse(window.buildConsoleStateInner('bridge-orders', state))).toEqual({});
  });

  it('does not select canonical Command by station name after an empty projection', () => {
    const state = {
      stationSystems: { command: ['command'] },
      systemConsoleFamilies: {},
      hasSystemConsoleFamilyProjection: true,
      blackboards: { command: { command_system_id: 'command' } },
    };
    expect(JSON.parse(window.buildConsoleStateInner('command', state))).toEqual({});
  });

  it('retains canonical Command only for the genuine pre-Welcome boot race', () => {
    const s = JSON.parse(window.buildConsoleStateInner('command', {}));
    expect(s.command_system_id).toBe('command');
  });

  it('folds an authored Dock host update into dirty routing and the Helm builder', () => {
    const state = new ClientSimState();
    state.apply({
      type: 'Welcome',
      data: {
        state: { phase: 'Lobby', players: [], complexity: {}, world: null },
        ship_stations: {},
        ship_config: {
          station_systems: { 'flight-control': ['berthing-clamps'] },
          system_console_families: { 'berthing-clamps': 'helm' },
        },
      },
    });
    const update = {
      type: 'BlackboardUpdate',
      data: {
        updates: [[
          'berthing-clamps',
          {
            kind: 'Dock',
            data: {
              range: 275,
              available: true,
              available_target: 'berth-1',
              available_target_name: 'world.berth.one',
              engaged: false,
              docked: false,
            },
          },
        ]],
      },
    };

    state.apply(update);

    expect(dirtyConsolesFor(
      update,
      state.stationSystems,
      state.systemConsoleFamilies,
    )).toEqual(new Set(['flight-control']));
    const payload = JSON.parse(window.buildConsoleStateInner('flight-control', state));
    expect(payload.dock).toMatchObject({
      system_id: 'berthing-clamps',
      range: 275,
      available: true,
      available_target: 'berth-1',
      available_target_name: 'world.berth.one',
    });
    expect(state.blackboards).not.toHaveProperty('dock');
  });

  it('falls back to the plain builder by name during the boot race (no stationSystems yet)', () => {
    const s = JSON.parse(window.buildConsoleStateInner('sensors', {}));
    expect(s).not.toHaveProperty('systems');
    expect(s).toHaveProperty('blips');
  });

  it("returns '{}' for names with no plain builder before Welcome (science, pilot)", () => {
    expect(JSON.parse(window.buildConsoleStateInner('science', {}))).toEqual({});
    expect(JSON.parse(window.buildConsoleStateInner('pilot', {}))).toEqual({});
  });

  // Acceptance criterion (issue #825): TOML can move a system between stations
  // without any console-state.js change — the routing and payload follow the
  // station_systems data alone.
  it('a system moved between stations in TOML re-routes with no code change', () => {
    const before = {
      stationSystems: {
        comms: ['navigation', 'comms'],
        tactical: ['tactical-radar', 'phaser-control'],
      },
    };
    const after = {
      stationSystems: {
        comms: ['comms'],
        tactical: ['tactical-radar', 'phaser-control', 'navigation'],
      },
    };
    const commsBefore = JSON.parse(window.buildConsoleStateInner('comms', before));
    expect(commsBefore.systems).toHaveProperty('navigation');
    // tactical is single-family before the move → flat weapons payload.
    const tacticalBefore = JSON.parse(window.buildConsoleStateInner('tactical', before));
    expect(tacticalBefore).not.toHaveProperty('systems');

    // After the TOML move: comms collapses to the flat comms payload, and
    // tactical becomes multi-family and picks up the navigation view.
    const commsAfter = JSON.parse(window.buildConsoleStateInner('comms', after));
    expect(commsAfter).not.toHaveProperty('systems');
    expect(commsAfter).toHaveProperty('messages');
    const tacticalAfter = JSON.parse(window.buildConsoleStateInner('tactical', after));
    expect(tacticalAfter.systems).toHaveProperty('navigation');
    expect(tacticalAfter.systems['navigation']).toHaveProperty('waypoint');
  });
});

describe('buildConsoleStateInner pilot routing', () => {
  // The 'pilot' station is dormant (no shipped TOML declares it) but must
  // route generically the moment a TOML re-declares it with multi-family
  // ownership — no hardcoded 'pilot' case remains.
  const PILOT_STATE = {
    stationSystems: {
      pilot: ['captain', 'helm-thrust', 'tactical-radar', 'blaster-fore', 'sensors', 'navigation', 'comms'],
    },
  };

  it("routes a multi-family 'pilot' station to the generic payload", () => {
    const s = JSON.parse(window.buildConsoleStateInner('pilot', PILOT_STATE));
    expect(s.station_id).toBe('pilot');
    for (const id of ['captain', 'helm-thrust', 'tactical-radar', 'sensors', 'navigation', 'comms']) {
      expect(s.systems).toHaveProperty(id);
    }
  });

  // Every nested view computes an own_hull for its own hardcoded station id
  // ('tactical', 'sensors', ...), which is meaningless on a hull with one
  // station. The top-level own_hull — the only one pilot.html reads — must be
  // the pilot's, or the footer damage bar shows the wrong systems.
  it('top-level own_hull aggregates the pilot station, not tactical', () => {
    const state = {
      stationSystems: { pilot: ['blaster-fore', 'helm-thrust'] },
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

describe('fine-System station routing', () => {
  it('builds the Courier Captain payload from its owned fine systems', () => {
    const state = {
      stationSystems: {
        captain: ['captain', 'viewscreen', 'red-alert', 'navigation', 'comms', 'shields-system', 'power-reactor', 'power-battery', 'repair'],
      },
    };
    const s = JSON.parse(window.buildConsoleStateInner('captain', state));
    expect(s.system_ids).toEqual(state.stationSystems.captain);
    expect(s.systems).toHaveProperty('red-alert');
    expect(s.systems).toHaveProperty('navigation');
    expect(s.systems).toHaveProperty('repair');
    expect(s.systems).not.toHaveProperty('helm-thrust');
  });

  it('builds the Courier Tactical payload from its owned fine systems', () => {
    const state = {
      stationSystems: {
        tactical: ['helm-thrust', 'helm-steering', 'tactical-radar', 'blaster-fore', 'sensors', 'sensor-radar'],
      },
    };
    const s = JSON.parse(window.buildConsoleStateInner('tactical', state));
    expect(s.systems).toHaveProperty('helm-thrust');
    expect(s.systems).toHaveProperty('tactical-radar');
    expect(s.systems).toHaveProperty('sensor-radar');
    expect(s.systems).not.toHaveProperty('repair');
  });
});
