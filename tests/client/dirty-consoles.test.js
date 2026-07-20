// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import {
  dirtyConsolesFor,
  STATIC_MESSAGE_CONSOLES,
  ALWAYS_PUSH,
} from '../../gui/dirty-consoles.js';

// Battleship-style identity stations: every station id names its console and
// owns fine system ids resolved by consoleForSystemId.
const BATTLESHIP_STATIONS = {
  captain: ['captain', 'viewscreen', 'red-alert'],
  helm: ['helm-thrust', 'helm-steering', 'helm-impulse', 'helm-boost', 'helm-lateral-thrust'],
  tactical: ['tactical-radar', 'phaser-control', 'phaser-fore', 'phaser-aft', 'torpedo-magazine'],
  repair: ['repair'],
  sensors: ['sensors', 'sensor-radar'],
  shields: ['shields-system', 'shield-arc-fore'],
  navigation: ['navigation'],
  power: ['power-reactor', 'power-battery'],
  comms: ['comms'],
};

// Courier-style composite: the whole ship on one 'pilot' station.
const COURIER_STATIONS = {
  pilot: [
    'captain', 'viewscreen', 'red-alert',
    'helm-thrust', 'helm-steering',
    'blaster-fore', 'tactical-radar',
    'sensors', 'navigation', 'comms',
  ],
};

// Destroyer-style composites: engineering = shields+power+repair,
// tactical = weapons+navigation+comms, captain = captain+sensors.
const DESTROYER_STATIONS = {
  captain: ['captain', 'viewscreen', 'red-alert', 'sensors', 'sensor-radar'],
  helm: ['helm-thrust', 'helm-steering', 'helm-lateral-thrust'],
  tactical: ['phaser-omni', 'torpedo-tube-fore', 'navigation', 'comms', 'tactical-radar'],
  engineering: ['shields-system', 'shield-arc-fore', 'power-reactor', 'power-battery', 'repair'],
};

const bbUpdate = ids => ({
  type: 'BlackboardUpdate',
  data: { updates: ids.map(id => [id, { kind: 'X', data: {} }]) },
});

describe('STATIC_MESSAGE_CONSOLES', () => {
  const expected = {
    Welcome: ['repair'],
    SimState: ['tactical', 'repair', 'sensors', 'navigation'],
    WorldSetup: ['tactical', 'helm', 'sensors', 'navigation'],
    EntitySpawned: ['tactical', 'helm', 'sensors', 'navigation'],
    AsteroidSpawned: ['tactical', 'helm', 'sensors', 'navigation'],
    TargetLock: ['tactical'],
    WeaponsUpdate: ['tactical'],
    // BeamStarted/BeamEnded removed in #825: sim-state no longer mutates on
    // them, so there is nothing to re-push.
    SystemHullUpdate: ['repair'],
    RepairState: ['repair'],
    PowerState: ['power'],
    ShieldStatus: ['shields'],
    AsteroidDestroyed: ['tactical', 'helm', 'sensors'],
    EntityDespawned: ['tactical', 'helm', 'sensors'],
    CommsState: ['comms'],
    RatingChanged: ['captain'],
  };

  for (const [type, consoles] of Object.entries(expected)) {
    it(`${type} dirties [${consoles.join(', ')}]`, () => {
      expect(dirtyConsolesFor({ type }, BATTLESHIP_STATIONS))
        .toEqual(new Set(consoles));
    });
  }

  it('BeamStarted / BeamEnded dirty nothing (removed in #825)', () => {
    expect(dirtyConsolesFor({ type: 'BeamStarted' }, BATTLESHIP_STATIONS)).toEqual(new Set());
    expect(dirtyConsolesFor({ type: 'BeamEnded' }, BATTLESHIP_STATIONS)).toEqual(new Set());
  });

  it('the table itself holds exactly the expected static entries', () => {
    expect(Object.keys(STATIC_MESSAGE_CONSOLES).sort())
      .toEqual(Object.keys(expected).sort());
  });
});

describe('dirtyConsolesFor — BlackboardUpdate on battleship (identity stations)', () => {
  // Today's server blackboard ids are coarse (one per console family).
  const coarse = ['helm', 'tactical', 'power', 'shields', 'repair', 'comms', 'sensors', 'navigation'];
  for (const id of coarse) {
    it(`coarse '${id}' lands on the identity console`, () => {
      expect(dirtyConsolesFor(bbUpdate([id]), BATTLESHIP_STATIONS))
        .toEqual(new Set([id]));
    });
  }

  it('fine ids resolve through the shared matcher (helm-thrust → helm)', () => {
    expect(dirtyConsolesFor(bbUpdate(['helm-thrust']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['helm']));
    expect(dirtyConsolesFor(bbUpdate(['phaser-fore']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['tactical']));
    expect(dirtyConsolesFor(bbUpdate(['power-battery']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['power']));
  });

  it('multiple updates in one message union their consoles', () => {
    expect(dirtyConsolesFor(bbUpdate(['power', 'shields']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['power', 'shields']));
  });

  it('unknown system id dirties nothing', () => {
    expect(dirtyConsolesFor(bbUpdate(['warp-core']), BATTLESHIP_STATIONS))
      .toEqual(new Set());
  });
});

describe('dirtyConsolesFor — captain/viewscreen currentView cascade', () => {
  // Helm/Sensors/Comms/Navigation derive their on-screen button state from
  // simState.currentView, so a captain or viewscreen change refreshes them.
  it('captain update cascades to helm/sensors/comms/navigation', () => {
    expect(dirtyConsolesFor(bbUpdate(['captain']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'sensors', 'comms', 'navigation']));
  });

  it('viewscreen update cascades identically', () => {
    expect(dirtyConsolesFor(bbUpdate(['viewscreen']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'sensors', 'comms', 'navigation']));
  });

  it('non-captain updates do not cascade', () => {
    expect(dirtyConsolesFor(bbUpdate(['shields']), BATTLESHIP_STATIONS))
      .toEqual(new Set(['shields']));
  });
});

describe('dirtyConsolesFor — composite stations route to the owning console', () => {
  it('courier: every update lands on pilot', () => {
    expect(dirtyConsolesFor(bbUpdate(['helm']), COURIER_STATIONS))
      .toEqual(new Set(['pilot']));
    expect(dirtyConsolesFor(bbUpdate(['tactical']), COURIER_STATIONS))
      .toEqual(new Set(['pilot']));
    // Cascade collapses onto the single owning console too.
    expect(dirtyConsolesFor(bbUpdate(['captain']), COURIER_STATIONS))
      .toEqual(new Set(['pilot']));
  });

  it('destroyer: shields/power/repair land on engineering', () => {
    for (const id of ['shields', 'power', 'repair']) {
      expect(dirtyConsolesFor(bbUpdate([id]), DESTROYER_STATIONS))
        .toEqual(new Set(['engineering']));
    }
  });

  it('destroyer: coarse tactical lands on the station owning phaser-*', () => {
    expect(dirtyConsolesFor(bbUpdate(['tactical']), DESTROYER_STATIONS))
      .toEqual(new Set(['tactical']));
  });

  it('destroyer: navigation/comms land on the tactical station, sensors on captain', () => {
    expect(dirtyConsolesFor(bbUpdate(['navigation']), DESTROYER_STATIONS))
      .toEqual(new Set(['tactical']));
    expect(dirtyConsolesFor(bbUpdate(['comms']), DESTROYER_STATIONS))
      .toEqual(new Set(['tactical']));
    expect(dirtyConsolesFor(bbUpdate(['sensors']), DESTROYER_STATIONS))
      .toEqual(new Set(['captain']));
  });

  it('destroyer: captain cascade fans out across the owning stations', () => {
    // captain+sensors → captain station; helm → helm; comms/navigation →
    // tactical station.
    expect(dirtyConsolesFor(bbUpdate(['captain']), DESTROYER_STATIONS))
      .toEqual(new Set(['captain', 'helm', 'tactical']));
  });
});

describe('dirtyConsolesFor — fallbacks and edge cases', () => {
  it('missing stationSystems falls back to coarse identity (boot race before Welcome)', () => {
    expect(dirtyConsolesFor(bbUpdate(['power']), null))
      .toEqual(new Set(['power']));
    expect(dirtyConsolesFor(bbUpdate(['captain']), undefined))
      .toEqual(new Set(['captain', 'helm', 'sensors', 'comms', 'navigation']));
  });

  it('empty stationSystems behaves like missing', () => {
    expect(dirtyConsolesFor(bbUpdate(['helm']), {}))
      .toEqual(new Set(['helm']));
  });

  it('a family no station owns falls back to its own name', () => {
    // Harmless: pushes to unmounted consoles are no-ops.
    const noShields = { helm: ['helm-thrust'] };
    expect(dirtyConsolesFor(bbUpdate(['shields']), noShields))
      .toEqual(new Set(['shields']));
  });

  it('unknown message type dirties nothing', () => {
    expect(dirtyConsolesFor({ type: 'PlayerJoined' }, BATTLESHIP_STATIONS)).toEqual(new Set());
    expect(dirtyConsolesFor({ type: 'NoSuchMessage' }, BATTLESHIP_STATIONS)).toEqual(new Set());
    expect(dirtyConsolesFor(null, BATTLESHIP_STATIONS)).toEqual(new Set());
    expect(dirtyConsolesFor({}, BATTLESHIP_STATIONS)).toEqual(new Set());
  });

  it('BlackboardUpdate with no updates dirties nothing', () => {
    expect(dirtyConsolesFor({ type: 'BlackboardUpdate', data: {} }, BATTLESHIP_STATIONS))
      .toEqual(new Set());
  });
});

describe('ALWAYS_PUSH', () => {
  it('captain is the only unconditionally-pushed console', () => {
    expect(ALWAYS_PUSH).toEqual(new Set(['captain']));
  });
});

describe('window exposure for the client.html inline script', () => {
  it('attaches dirtyConsolesFor and DIRTY_ALWAYS_PUSH to window', () => {
    expect(typeof window.dirtyConsolesFor).toBe('function');
    expect(window.DIRTY_ALWAYS_PUSH).toBe(ALWAYS_PUSH);
  });
});
