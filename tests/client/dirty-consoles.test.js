// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import {
  dirtyConsolesFor,
  ALWAYS_PUSH_FAMILIES,
  alwaysPushConsoles,
} from '../../gui/dirty-consoles.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { ClientCommsState } from '../../gui/comms-state.js';
import { LobbyState } from '../../gui/lobby-state.js';
import { emptyReducerResult, mergeReducerResults } from '../../gui/reducer-result.js';

const BATTLESHIP_STATIONS = {
  captain: ['captain', 'viewscreen', 'red-alert'],
  helm: ['helm-thrust', 'helm-steering'],
  tactical: ['tactical-radar', 'phaser-fore'],
  repair: ['repair'], sensors: ['sensors'], shields: ['shields-system'],
  navigation: ['navigation'], power: ['power-reactor'], comms: ['comms'],
};
const DESTROYER_STATIONS = {
  captain: ['captain', 'sensors'],
  helm: ['helm-thrust'],
  tactical: ['phaser-omni', 'navigation', 'comms'],
  engineering: ['shields-system', 'power-reactor', 'repair'],
};
const COURIER_STATIONS = {
  pilot: ['captain', 'helm-thrust', 'blaster-fore', 'sensors', 'navigation', 'comms'],
};

const SYSTEM_FAMILIES = {
  captain: 'captain', viewscreen: 'captain', 'red-alert': 'captain',
  'helm-thrust': 'helm', 'helm-steering': 'helm',
  'tactical-radar': 'tactical', 'phaser-fore': 'tactical',
  'phaser-omni': 'tactical', 'blaster-fore': 'tactical',
  repair: 'repair', sensors: 'sensors', 'shields-system': 'shields',
  navigation: 'navigation', 'power-reactor': 'power', comms: 'comms',
};
const BLACKBOARD_FAMILIES = {
  helm: 'helm', tactical: 'tactical', power: 'power', shields: 'shields',
  dossiers: 'comms', scan: 'sensors',
};

const bbUpdate = ids => ({
  type: 'BlackboardUpdate',
  data: { updates: ids.map(id => [id, { kind: 'X', data: {} }]) },
});
const route = (changes, stations = BATTLESHIP_STATIONS, systems = SYSTEM_FAMILIES,
  blackboards = BLACKBOARD_FAMILIES) => (
  dirtyConsolesFor(changes, stations, systems, blackboards)
);

const reduce = msg => mergeReducerResults(
  new LobbyState().apply(msg),
  new ClientSimState().apply(msg),
  new ClientCommsState().apply(msg),
);

describe('semantic domain routing', () => {
  const cases = [
    ['Welcome', { type: 'Welcome' }, ['repair']],
    ['SimState', { type: 'SimState' }, ['tactical', 'repair', 'sensors', 'navigation']],
    ['WorldSetup', { type: 'WorldSetup' }, ['tactical', 'helm', 'sensors', 'navigation']],
    ['EntitySpawned', {
      type: 'EntitySpawned', data: { snapshot: { uuid: 'ship-1' } },
    }, ['tactical', 'helm', 'sensors', 'navigation']],
    ['AsteroidSpawned', {
      type: 'AsteroidSpawned', data: { uuid: 'rock-1', x: 0, y: 0, z: 0 },
    }, ['tactical', 'helm', 'sensors', 'navigation']],
    ['TargetLock', { type: 'TargetLock' }, ['tactical']],
    ['WeaponsUpdate', { type: 'WeaponsUpdate' }, ['tactical']],
    ['SystemHullUpdate', { type: 'SystemHullUpdate' }, ['repair']],
    ['RepairState', { type: 'RepairState' }, ['repair']],
    ['PowerState', { type: 'PowerState' }, ['power']],
    ['ShieldStatus', { type: 'ShieldStatus' }, ['shields']],
    ['AsteroidDestroyed', { type: 'AsteroidDestroyed' }, ['tactical', 'helm', 'sensors']],
    ['EntityDespawned', { type: 'EntityDespawned' }, ['tactical', 'helm', 'sensors']],
    ['CommsState', { type: 'CommsState' }, ['comms']],
    ['CommsResponseRejected', { type: 'CommsResponseRejected' }, ['comms']],
    ['RatingChanged', {
      type: 'RatingChanged', data: { station_id: 'captain', rating_name: 'Std' },
    }, ['captain']],
  ];

  for (const [name, msg, consoles] of cases) {
    it(`${name} reducer result dirties [${consoles.join(', ')}]`, () => {
      expect(route(reduce(msg))).toEqual(new Set(consoles));
    });
  }

  it('routes semantic domains to composite Station ids through actual ownership', () => {
    expect(route(reduce({ type: 'SimState' }), DESTROYER_STATIONS))
      .toEqual(new Set(['tactical', 'engineering', 'captain']));
    expect(route(reduce({ type: 'WorldSetup' }), COURIER_STATIONS))
      .toEqual(new Set(['pilot']));
  });

  it('does not invent Station ids before Welcome', () => {
    expect(route(reduce({ type: 'SimState' }), null)).toEqual(new Set());
  });

  it('has no ServerMessage input or replacement message census', () => {
    const changes = reduce({ type: 'WeaponsUpdate' });
    expect(dirtyConsolesFor(
      changes,
      BATTLESHIP_STATIONS,
      SYSTEM_FAMILIES,
      BLACKBOARD_FAMILIES,
    )).toEqual(new Set(['tactical']));
    expect(route(reduce({ type: 'NoSuchMessage' }))).toEqual(new Set());
  });
});

describe('System and Blackboard metadata routing', () => {
  it('routes changed Systems through the complete System projection', () => {
    const changes = emptyReducerResult();
    changes.changedSystems.add('berthing-clamps-alpha');
    const stations = { 'flight-control': ['berthing-clamps-alpha'] };
    const families = { 'berthing-clamps-alpha': 'helm' };
    expect(route(changes, stations, families)).toEqual(new Set(['flight-control']));
  });

  it('routes reserved aggregate keys through their separate projection', () => {
    expect(route(reduce(bbUpdate(['helm'])))).toEqual(new Set(['helm']));
    expect(route(reduce(bbUpdate(['power', 'shields']))))
      .toEqual(new Set(['power', 'shields']));
    expect(route(reduce(bbUpdate(['scan'])))).toEqual(new Set(['sensors']));
  });

  it('routes actual System ids through the complete System projection', () => {
    expect(route(reduce(bbUpdate(['helm-thrust'])))).toEqual(new Set(['helm']));
    expect(route(reduce(bbUpdate(['phaser-fore'])))).toEqual(new Set(['tactical']));
  });

  it('routes an arbitrary instance id without spelling inference', () => {
    const stations = { 'flight-control': ['berthing-clamps-alpha'] };
    const families = { 'berthing-clamps-alpha': 'helm' };
    expect(route(reduce(bbUpdate(['berthing-clamps-alpha'])), stations, families))
      .toEqual(new Set(['flight-control']));
  });

  it('never guesses a family from an exact id or prefix', () => {
    expect(route(reduce(bbUpdate(['helm-thrust'])), BATTLESHIP_STATIONS, {}))
      .toEqual(new Set());
    expect(route(reduce(bbUpdate(['phaser-surprise'])), BATTLESHIP_STATIONS, {}))
      .toEqual(new Set());
  });

  it('never treats a reserved channel as a System', () => {
    expect(route(reduce(bbUpdate(['scan'])), BATTLESHIP_STATIONS, SYSTEM_FAMILIES, {}))
      .toEqual(new Set());
  });

  it('routes composite Stations from actual ownership', () => {
    for (const [id, stations, expected] of [
      ['shields', DESTROYER_STATIONS, ['engineering']],
      ['navigation', DESTROYER_STATIONS, ['tactical']],
      ['sensors', DESTROYER_STATIONS, ['captain']],
      ['tactical', COURIER_STATIONS, ['pilot']],
    ]) {
      expect(route(reduce(bbUpdate([id])), stations)).toEqual(new Set(expected));
    }
  });

  it('cascades Captain-family Blackboard changes through current-view consumers', () => {
    const changes = reduce(bbUpdate(['captain']));
    expect(route(changes, BATTLESHIP_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'sensors', 'comms', 'navigation']));
    expect(route(changes, DESTROYER_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'tactical']));
  });

  it('routes solely from reducer results after the original payload is gone', () => {
    const msg = bbUpdate(['scan']);
    const changes = reduce(msg);
    msg.data.updates = [];
    expect(route(changes)).toEqual(new Set(['sensors']));
  });

  it('has no station-name boot fallback before Welcome', () => {
    expect(route(reduce(bbUpdate(['power'])), null)).toEqual(new Set());
    expect(route(reduce(bbUpdate(['captain'])), {})).toEqual(new Set());
  });

  it('ignores unknown ids, unknown domains, and empty updates', () => {
    expect(route(reduce(bbUpdate(['warp-core'])))).toEqual(new Set());
    expect(route(reduce({ type: 'BlackboardUpdate', data: {} }))).toEqual(new Set());
    const unknown = emptyReducerResult();
    unknown.changedDomains.add('future-domain');
    unknown.changedSystems.add('future-system');
    expect(route(unknown)).toEqual(new Set());
  });
});

describe('human-seeking Systems', () => {
  const seeking = (id, host) => ({
    type: 'BlackboardUpdate',
    data: { updates: [[id, { kind: 'Comms', data: { host_station: host } }]] },
  });

  it('dirties every known Station when a visiting System appears or disappears', () => {
    const expected = new Set(['captain', 'helm', 'tactical', 'engineering']);
    expect(route(reduce(seeking('comms', 'engineering')), DESTROYER_STATIONS))
      .toEqual(expected);
    expect(route(reduce(seeking('comms', null)), DESTROYER_STATIONS))
      .toEqual(expected);
  });

  it('does not invent Stations when topology is unknown', () => {
    expect(route(reduce(seeking('comms', 'engineering')), null)).toEqual(new Set());
  });
});

describe('exports', () => {
  it('projects the unconditional Captain family to actual owning Stations', () => {
    expect(ALWAYS_PUSH_FAMILIES).toEqual(new Set(['captain']));
    expect(alwaysPushConsoles(BATTLESHIP_STATIONS, SYSTEM_FAMILIES))
      .toEqual(new Set(['captain']));
    expect(alwaysPushConsoles(COURIER_STATIONS, SYSTEM_FAMILIES))
      .toEqual(new Set(['pilot']));
    expect(typeof window.dirtyConsolesFor).toBe('function');
    expect(window.alwaysPushConsoles).toBe(alwaysPushConsoles);
  });
});
