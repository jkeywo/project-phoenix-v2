// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import '../../gui/console-state.js';
import { aggregateStationHull } from '../../gui/console-state.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { dirtyConsolesFor } from '../../gui/dirty-consoles.js';

const FAMILY_BY_ID = {
  captain: 'captain', viewscreen: 'captain', 'red-alert': 'captain',
  'helm-thrust': 'helm', 'helm-steering': 'helm',
  'tactical-radar': 'tactical', 'phaser-control': 'tactical', 'phaser-fore': 'tactical',
  'blaster-fore': 'tactical', sensors: 'sensors', 'sensor-radar': 'sensors',
  'shields-system': 'shields', 'power-reactor': 'power', 'power-battery': 'power',
  repair: 'repair', navigation: 'navigation', comms: 'comms',
};

function stateFor(stationSystems, extra = {}) {
  const ids = Object.values(stationSystems).flat();
  return {
    stationSystems,
    systemConsoleFamilies: Object.fromEntries(
      ids.filter(id => FAMILY_BY_ID[id]).map(id => [id, FAMILY_BY_ID[id]]),
    ),
    ...extra,
  };
}

describe('buildConsoleStateInner metadata routing', () => {
  const representative = [
    ['engineering', ['shields-system', 'power-reactor', 'repair'], ['shields', 'power', 'repair']],
    ['science', ['sensors', 'shields-system'], ['sensors', 'shields']],
    ['comms', ['navigation', 'comms'], ['navigation', 'comms']],
    ['captain', ['captain', 'sensors'], ['captain', 'sensors']],
    ['tactical', ['tactical-radar', 'navigation', 'comms'], ['tactical', 'navigation', 'comms']],
  ];

  for (const [station, ids, families] of representative) {
    it(`builds representative multi-family ${station} from actual owned ids`, () => {
      const state = stateFor({ [station]: ids });
      const payload = JSON.parse(window.buildConsoleStateInner(station, state));
      expect(payload.station_id).toBe(station);
      expect(payload.system_ids).toEqual(ids);
      expect(Object.keys(payload.systems)).toEqual(ids);
      expect(new Set(Object.values(payload.system_families))).toEqual(new Set(families));
    });
  }

  it('keeps a single-family Station flat while carrying its actual ids and projection', () => {
    const state = stateFor({ gunnery: ['tactical-radar', 'phaser-fore'] });
    const payload = JSON.parse(window.buildConsoleStateInner('gunnery', state));
    expect(payload).not.toHaveProperty('systems');
    expect(payload).toHaveProperty('banks');
    expect(payload.system_ids).toEqual(['tactical-radar', 'phaser-fore']);
    expect(payload.system_families).toEqual({
      'tactical-radar': 'tactical', 'phaser-fore': 'tactical',
    });
  });

  it('selects Command by metadata and typed blackboard with arbitrary names', () => {
    const state = {
      stationSystems: { 'bridge-orders': ['orders-array-alpha'] },
      systemConsoleFamilies: { 'orders-array-alpha': 'command' },
      blackboards: {
        'orders-array-alpha': {
          command_system_id: 'orders-array-alpha',
          directed_station: 'tactical',
          directed_station_name: 'Tactical',
          directed_station_ai: true,
          stances: [],
        },
      },
      blackboardKinds: { 'orders-array-alpha': 'Command' },
    };
    const payload = JSON.parse(window.buildConsoleStateInner('bridge-orders', state));
    expect(payload.command_system_id).toBe('orders-array-alpha');
    expect(payload.directed_station).toBe('tactical');
    expect(payload.system_ids).toEqual(['orders-array-alpha']);
  });

  it('uses the common family registry for arbitrary Tractor and Umbilical instances', () => {
    const state = {
      stationSystems: { engineering: ['tow-array-alpha', 'cargo-link-port'] },
      systemConsoleFamilies: {
        'tow-array-alpha': 'tractor',
        'cargo-link-port': 'umbilical',
      },
      blackboards: {
        'tow-array-alpha': { engaged: true, coupled_target: 'freighter-1' },
        'cargo-link-port': { running: true, rate: 12 },
      },
      blackboardKinds: {
        'tow-array-alpha': 'Tractor',
        'cargo-link-port': 'Umbilical',
      },
    };
    const payload = JSON.parse(window.buildConsoleStateInner('engineering', state));
    expect(payload.systems['tow-array-alpha']).toMatchObject({
      system_id: 'tow-array-alpha', engaged: true,
    });
    expect(payload.systems['cargo-link-port']).toMatchObject({
      system_id: 'cargo-link-port', running: true, rate: 12,
    });
  });

  it('builds a wholly arbitrary multi-family Station without id inference', () => {
    const state = {
      stationSystems: { operations: ['glass-eye-seven', 'quiet-line-port'] },
      systemConsoleFamilies: {
        'glass-eye-seven': 'sensors',
        'quiet-line-port': 'comms',
      },
      blackboards: {
        'glass-eye-seven': { radar_range: 777 },
        'quiet-line-port': { messages: [{ id: 'm1' }], contacts: [] },
      },
      blackboardKinds: {
        'glass-eye-seven': 'Sensors',
        'quiet-line-port': 'Comms',
      },
    };
    const payload = JSON.parse(window.buildConsoleStateInner('operations', state));
    expect(payload.systems['glass-eye-seven'].scan_range).toBe(777);
    expect(payload.systems['quiet-line-port'].messages).toEqual([{ id: 'm1' }]);
    expect(payload.system_families).toEqual(state.systemConsoleFamilies);
  });

  it('does not infer an unmapped family from an exact id, prefix, or Station name', () => {
    expect(JSON.parse(window.buildConsoleStateInner('command', {
      stationSystems: { command: ['command'] }, systemConsoleFamilies: {},
    }))).toEqual({});
    expect(JSON.parse(window.buildConsoleStateInner('tactical', {
      stationSystems: { tactical: ['phaser-surprise'] }, systemConsoleFamilies: {},
    }))).toEqual({});
  });

  it('has no Station-name builder switch before Welcome', () => {
    expect(JSON.parse(window.buildConsoleStateInner('captain', {}))).toEqual({});
    expect(JSON.parse(window.buildConsoleStateInner('sensors', {}))).toEqual({});
    expect(JSON.parse(window.buildConsoleStateInner('command', {}))).toEqual({});
  });

  it('follows a System moved between Stations with no code change', () => {
    const before = stateFor({ comms: ['navigation', 'comms'], tactical: ['tactical-radar'] });
    const after = stateFor({ comms: ['comms'], tactical: ['tactical-radar', 'navigation'] });
    expect(JSON.parse(window.buildConsoleStateInner('comms', before)).systems)
      .toHaveProperty('navigation');
    expect(JSON.parse(window.buildConsoleStateInner('tactical', before))).not.toHaveProperty('systems');
    expect(JSON.parse(window.buildConsoleStateInner('comms', after))).not.toHaveProperty('systems');
    expect(JSON.parse(window.buildConsoleStateInner('tactical', after)).systems)
      .toHaveProperty('navigation');
  });
});

describe('Dock tracer and visiting Systems', () => {
  it('folds an arbitrary Dock id into dirty routing and Helm state', () => {
    const state = new ClientSimState();
    state.apply({
      type: 'Welcome',
      data: {
        state: { phase: 'Lobby', players: [], complexity: {}, world: null },
        ship_stations: {},
        ship_config: {
          station_systems: { 'flight-control': ['berthing-clamps'] },
          system_console_families: { 'berthing-clamps': 'helm' },
          blackboard_console_families: { helm: 'helm' },
        },
      },
    });
    const update = {
      type: 'BlackboardUpdate',
      data: { updates: [[
        'berthing-clamps',
        { kind: 'Dock', data: { range: 275, available: true, docked: false } },
      ]] },
    };
    const changes = state.apply(update);
    expect(dirtyConsolesFor(
      changes,
      state.stationSystems,
      state.systemConsoleFamilies,
      state.blackboardConsoleFamilies,
    )).toEqual(new Set(['flight-control']));
    expect(JSON.parse(window.buildConsoleStateInner('flight-control', state)).dock)
      .toMatchObject({ system_id: 'berthing-clamps', range: 275, available: true });
  });

  it('adds a visiting arbitrary System under its actual id and metadata', () => {
    const state = {
      stationSystems: { bridge: ['bridge-core'], remote: ['far-comms-array'] },
      systemConsoleFamilies: { 'bridge-core': 'captain', 'far-comms-array': 'comms' },
      blackboards: { 'far-comms-array': { host_station: 'bridge', messages: [] } },
      blackboardKinds: { 'far-comms-array': 'Comms' },
    };
    const payload = JSON.parse(window.buildConsoleState('bridge', state));
    expect(payload.systems).toHaveProperty('far-comms-array');
    expect(payload.system_families['far-comms-array']).toBe('comms');
    expect(payload.hosted_systems).toEqual(['bridge-core', 'far-comms-array']);
  });
});

describe('station hull normalization', () => {
  it('uses the actual Station ownership for the top-level aggregate', () => {
    const state = {
      stationSystems: { pilot: ['odd-blaster-id', 'odd-engine-id'] },
      systemConsoleFamilies: { 'odd-blaster-id': 'tactical', 'odd-engine-id': 'helm' },
      consoleHull: [{ system_id: 'odd-blaster-id', current: 6, max_hp: 12 }],
    };
    const payload = JSON.parse(window.buildConsoleState('pilot', state));
    expect(payload.own_hull).toEqual(JSON.parse(JSON.stringify(
      aggregateStationHull('pilot', state.consoleHull, state.stationSystems),
    )));
    expect(payload.own_hull.entries.map(entry => entry.system_id)).toEqual(['odd-blaster-id']);
  });
});
