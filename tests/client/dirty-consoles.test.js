// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import {
  dirtyConsolesFor,
  STATIC_MESSAGE_FAMILIES,
  ALWAYS_PUSH_FAMILIES,
  alwaysPushConsoles,
} from '../../gui/dirty-consoles.js';

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
const route = (msg, stations = BATTLESHIP_STATIONS, systems = SYSTEM_FAMILIES,
  blackboards = BLACKBOARD_FAMILIES) => (
  dirtyConsolesFor(msg, stations, systems, blackboards)
);

describe('STATIC_MESSAGE_FAMILIES', () => {
  const expected = {
    Welcome: ['repair'],
    SimState: ['tactical', 'repair', 'sensors', 'navigation'],
    WorldSetup: ['tactical', 'helm', 'sensors', 'navigation'],
    EntitySpawned: ['tactical', 'helm', 'sensors', 'navigation'],
    AsteroidSpawned: ['tactical', 'helm', 'sensors', 'navigation'],
    TargetLock: ['tactical'], WeaponsUpdate: ['tactical'],
    SystemHullUpdate: ['repair'], RepairState: ['repair'], PowerState: ['power'],
    ShieldStatus: ['shields'],
    AsteroidDestroyed: ['tactical', 'helm', 'sensors'],
    EntityDespawned: ['tactical', 'helm', 'sensors'],
    CommsState: ['comms'], CommsResponseRejected: ['comms'], RatingChanged: ['captain'],
  };
  for (const [type, consoles] of Object.entries(expected)) {
    it(`${type} dirties [${consoles.join(', ')}]`, () => {
      expect(route({ type })).toEqual(new Set(consoles));
    });
  }
  it('holds exactly the expected static entries', () => {
    expect(Object.keys(STATIC_MESSAGE_FAMILIES).sort()).toEqual(Object.keys(expected).sort());
  });

  it('routes static families to composite Station ids through actual ownership', () => {
    expect(route({ type: 'SimState' }, DESTROYER_STATIONS))
      .toEqual(new Set(['tactical', 'engineering', 'captain']));
    expect(route({ type: 'WorldSetup' }, COURIER_STATIONS))
      .toEqual(new Set(['pilot']));
  });

  it('does not invent static Station ids before Welcome', () => {
    expect(route({ type: 'SimState' }, null)).toEqual(new Set());
  });
});

describe('BlackboardUpdate metadata routing', () => {
  it('routes reserved aggregate keys through their separate projection', () => {
    expect(route(bbUpdate(['helm']))).toEqual(new Set(['helm']));
    expect(route(bbUpdate(['power', 'shields']))).toEqual(new Set(['power', 'shields']));
    expect(route(bbUpdate(['scan']))).toEqual(new Set(['sensors']));
  });

  it('routes actual System ids through the complete System projection', () => {
    expect(route(bbUpdate(['helm-thrust']))).toEqual(new Set(['helm']));
    expect(route(bbUpdate(['phaser-fore']))).toEqual(new Set(['tactical']));
  });

  it('routes an arbitrary instance id without spelling inference', () => {
    const stations = { 'flight-control': ['berthing-clamps-alpha'] };
    const families = { 'berthing-clamps-alpha': 'helm' };
    expect(route(bbUpdate(['berthing-clamps-alpha']), stations, families))
      .toEqual(new Set(['flight-control']));
  });

  it('never guesses a family from an exact id or prefix', () => {
    expect(route(bbUpdate(['helm-thrust']), BATTLESHIP_STATIONS, {})).toEqual(new Set());
    expect(route(bbUpdate(['phaser-surprise']), BATTLESHIP_STATIONS, {})).toEqual(new Set());
  });

  it('never treats a reserved channel as a System', () => {
    expect(route(bbUpdate(['scan']), BATTLESHIP_STATIONS, SYSTEM_FAMILIES, {}))
      .toEqual(new Set());
  });

  it('routes composite Stations from actual ownership', () => {
    expect(route(bbUpdate(['shields']), DESTROYER_STATIONS)).toEqual(new Set(['engineering']));
    expect(route(bbUpdate(['navigation']), DESTROYER_STATIONS)).toEqual(new Set(['tactical']));
    expect(route(bbUpdate(['sensors']), DESTROYER_STATIONS)).toEqual(new Set(['captain']));
    expect(route(bbUpdate(['tactical']), COURIER_STATIONS)).toEqual(new Set(['pilot']));
  });

  it('cascades Captain-family changes through current-view consumers', () => {
    expect(route(bbUpdate(['captain']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'sensors', 'comms', 'navigation']));
    expect(route(bbUpdate(['captain']), DESTROYER_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'tactical']));
  });

  it('has no station-name boot fallback before Welcome', () => {
    expect(route(bbUpdate(['power']), null)).toEqual(new Set());
    expect(route(bbUpdate(['captain']), {})).toEqual(new Set());
  });

  it('ignores unknown messages, unknown ids, and empty updates', () => {
    expect(route({ type: 'NoSuchMessage' })).toEqual(new Set());
    expect(route(bbUpdate(['warp-core']))).toEqual(new Set());
    expect(route({ type: 'BlackboardUpdate', data: {} })).toEqual(new Set());
  });
});

describe('human-seeking Systems', () => {
  const seeking = (id, host) => ({
    type: 'BlackboardUpdate',
    data: { updates: [[id, { kind: 'Comms', data: { host_station: host } }]] },
  });

  it('dirties every known Station when a host appears or disappears', () => {
    const expected = new Set(['captain', 'helm', 'tactical', 'engineering']);
    expect(route(seeking('comms', 'engineering'), DESTROYER_STATIONS)).toEqual(expected);
    expect(route(seeking('comms', null), DESTROYER_STATIONS)).toEqual(expected);
  });

  it('does not invent Stations when topology is unknown', () => {
    expect(route(seeking('comms', 'engineering'), null)).toEqual(new Set());
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
