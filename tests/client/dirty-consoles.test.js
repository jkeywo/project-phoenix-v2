// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import {
  dirtyConsolesFor,
  STATIC_MESSAGE_FAMILIES,
  ALWAYS_PUSH_FAMILIES,
  alwaysPushConsoles,
} from '../../gui/dirty-consoles.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { emptyReducerResult } from '../../gui/reducer-result.js';

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
  blackboards = BLACKBOARD_FAMILIES, changes = emptyReducerResult()) => (
  dirtyConsolesFor(msg, changes, stations, systems, blackboards)
);

const reduce = msg => new ClientSimState().apply(msg);

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
    const helm = bbUpdate(['helm']);
    const powerShields = bbUpdate(['power', 'shields']);
    const scan = bbUpdate(['scan']);
    expect(route(helm, undefined, undefined, undefined, reduce(helm))).toEqual(new Set(['helm']));
    expect(route(powerShields, undefined, undefined, undefined, reduce(powerShields)))
      .toEqual(new Set(['power', 'shields']));
    expect(route(scan, undefined, undefined, undefined, reduce(scan))).toEqual(new Set(['sensors']));
  });

  it('routes actual System ids through the complete System projection', () => {
    const helm = bbUpdate(['helm-thrust']);
    const phaser = bbUpdate(['phaser-fore']);
    expect(route(helm, undefined, undefined, undefined, reduce(helm))).toEqual(new Set(['helm']));
    expect(route(phaser, undefined, undefined, undefined, reduce(phaser)))
      .toEqual(new Set(['tactical']));
  });

  it('routes an arbitrary instance id without spelling inference', () => {
    const stations = { 'flight-control': ['berthing-clamps-alpha'] };
    const families = { 'berthing-clamps-alpha': 'helm' };
    const msg = bbUpdate(['berthing-clamps-alpha']);
    expect(route(msg, stations, families, undefined, reduce(msg)))
      .toEqual(new Set(['flight-control']));
  });

  it('never guesses a family from an exact id or prefix', () => {
    const helm = bbUpdate(['helm-thrust']);
    const phaser = bbUpdate(['phaser-surprise']);
    expect(route(helm, BATTLESHIP_STATIONS, {}, undefined, reduce(helm))).toEqual(new Set());
    expect(route(phaser, BATTLESHIP_STATIONS, {}, undefined, reduce(phaser)))
      .toEqual(new Set());
  });

  it('never treats a reserved channel as a System', () => {
    const msg = bbUpdate(['scan']);
    expect(route(msg, BATTLESHIP_STATIONS, SYSTEM_FAMILIES, {}, reduce(msg)))
      .toEqual(new Set());
  });

  it('routes composite Stations from actual ownership', () => {
    for (const [id, stations, expected] of [
      ['shields', DESTROYER_STATIONS, ['engineering']],
      ['navigation', DESTROYER_STATIONS, ['tactical']],
      ['sensors', DESTROYER_STATIONS, ['captain']],
      ['tactical', COURIER_STATIONS, ['pilot']],
    ]) {
      const msg = bbUpdate([id]);
      expect(route(msg, stations, undefined, undefined, reduce(msg))).toEqual(new Set(expected));
    }
  });

  it('cascades Captain-family changes through current-view consumers', () => {
    const msg = bbUpdate(['captain']);
    expect(route(msg, BATTLESHIP_STATIONS, undefined, undefined, reduce(msg)))
      .toEqual(new Set(['captain', 'helm', 'sensors', 'comms', 'navigation']));
    expect(route(msg, DESTROYER_STATIONS, undefined, undefined, reduce(msg)))
      .toEqual(new Set(['captain', 'helm', 'tactical']));
  });

  it('routes from the reducer result after the original payload is gone', () => {
    const msg = bbUpdate(['scan']);
    const changes = reduce(msg);
    msg.data.updates = [];
    expect(route(msg, undefined, undefined, undefined, changes)).toEqual(new Set(['sensors']));
  });

  it('has no station-name boot fallback before Welcome', () => {
    const power = bbUpdate(['power']);
    const captain = bbUpdate(['captain']);
    expect(route(power, null, undefined, undefined, reduce(power))).toEqual(new Set());
    expect(route(captain, {}, undefined, undefined, reduce(captain))).toEqual(new Set());
  });

  it('ignores unknown messages, unknown ids, and empty updates', () => {
    expect(route({ type: 'NoSuchMessage' })).toEqual(new Set());
    const unknown = bbUpdate(['warp-core']);
    expect(route(unknown, undefined, undefined, undefined, reduce(unknown))).toEqual(new Set());
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
    const appeared = seeking('comms', 'engineering');
    const disappeared = seeking('comms', null);
    expect(route(appeared, DESTROYER_STATIONS, undefined, undefined, reduce(appeared)))
      .toEqual(expected);
    expect(route(disappeared, DESTROYER_STATIONS, undefined, undefined, reduce(disappeared)))
      .toEqual(expected);
  });

  it('does not invent Stations when topology is unknown', () => {
    const msg = seeking('comms', 'engineering');
    expect(route(msg, null, undefined, undefined, reduce(msg))).toEqual(new Set());
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
