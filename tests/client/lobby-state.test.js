import { describe, it, expect } from 'vitest';
import {
  LobbyState, lobbyState, reconcileActiveConsole, consolesOf, ALL_CONSOLES,
} from '../../gui/lobby-state.js';

function p(token, name, consoles) {
  return { token, name, consoles, connected: true };
}

function welcome(state, shipStations, shipConfig) {
  return {
    type: 'Welcome',
    data: {
      state,
      ship_stations: shipStations || { stations: [] },
      ship_config: shipConfig || {},
    },
  };
}

// New B3 shape: flat stations array, no configs/min_players/max_players.
const TWO_STATION_SHIP = {
  stations: [
    { name: 'Helm', description: 'Pilot', rank: '', short_code: 'HLM', consoles: ['Helm', 'CaptainChair'] },
    { name: 'Tactical', description: 'Weapons', rank: '', short_code: 'TAC', consoles: ['Tactical'] },
  ],
};

describe('LobbyState defaults', () => {
  it('starts empty in Lobby phase', () => {
    const s = new LobbyState();
    expect(s.phase).toBe('Lobby');
    expect(s.players).toEqual([]);
    expect(s.gameOverReason).toBeNull();
  });

  it('exports a singleton and the canonical console list', () => {
    expect(lobbyState).toBeInstanceOf(LobbyState);
    expect(ALL_CONSOLES).toHaveLength(9);
    expect(ALL_CONSOLES[0]).toBe('CaptainChair');
  });
});

describe('apply Welcome', () => {
  it('replaces state wholesale', () => {
    const s = new LobbyState();
    s.players.push(p('ghost', 'Ghost', []));
    s.apply(welcome({
      phase: 'Lobby',
      players: [p('a', 'Alice', ['CaptainChair'])],
      world: null,
    }));
    expect(s.players).toHaveLength(1);
    expect(s.players[0].name).toBe('Alice');
  });

  it('extracts scenario title and body from world', () => {
    const s = new LobbyState();
    s.apply(welcome({
      phase: 'InProgress',
      players: [],
      world: { entities: [], scenario_title: 'Distress Call', scenario_description: 'Save them' },
    }));
    expect(s.scenarioTitle).toBe('Distress Call');
    expect(s.scenarioBody).toBe('Save them');
    expect(s.phase).toBe('InProgress');
  });

  it('stores ship stations and ship config', () => {
    const s = new LobbyState();
    s.apply(welcome({ phase: 'Lobby', players: [] },
      TWO_STATION_SHIP, { repair_team_count: 3 }));
    expect(s.shipStations).toEqual(TWO_STATION_SHIP);
    expect(s.shipConfig.repair_team_count).toBe(3);
  });
});

describe('apply player lifecycle', () => {
  it('PlayerJoined appends new players in order', () => {
    const s = new LobbyState();
    s.apply({ type: 'PlayerJoined', data: { player: p('a', 'Alice', []) } });
    s.apply({ type: 'PlayerJoined', data: { player: p('b', 'Bob', []) } });
    expect(s.players.map(x => x.token)).toEqual(['a', 'b']);
  });

  it('PlayerJoined with existing token replaces in place', () => {
    const s = new LobbyState();
    s.apply({ type: 'PlayerJoined', data: { player: p('a', 'Alice', ['Helm']) } });
    s.apply({ type: 'PlayerJoined', data: { player: p('a', 'Alice2', []) } });
    expect(s.players).toHaveLength(1);
    expect(s.players[0].name).toBe('Alice2');
    expect(s.players[0].consoles).toEqual([]);
  });

  it('PlayerLeft removes by token', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', []), p('b', 'Bob', [])];
    s.apply({ type: 'PlayerLeft', data: { token: 'a' } });
    expect(s.players.map(x => x.token)).toEqual(['b']);
  });

  it('NameChanged updates only the named player', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', []), p('b', 'Bob', [])];
    s.apply({ type: 'NameChanged', data: { token: 'a', name: 'Alicia' } });
    expect(s.players[0].name).toBe('Alicia');
    expect(s.players[1].name).toBe('Bob');
  });
});

describe('apply StationAssigned', () => {
  it('assigns consoles to the named player', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', []), p('b', 'Bob', [])];
    s.apply({ type: 'StationAssigned', data: { token: 'a', station: 'Captain', consoles: ['CaptainChair'] } });
    expect(s.players[0].consoles).toEqual(['CaptainChair']);
    expect(s.players[1].consoles).toEqual([]);
  });

  it('steals consoles from the previous holder first', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', ['Helm']), p('b', 'Bob', [])];
    s.apply({ type: 'StationAssigned', data: { token: 'b', station: 'Helm', consoles: ['Helm'] } });
    expect(s.players[0].consoles).toEqual([]);
    expect(s.players[1].consoles).toEqual(['Helm']);
  });

  it('spectator assignment (empty consoles) clears the player consoles', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', ['CaptainChair', 'Helm'])];
    s.apply({ type: 'StationAssigned', data: { token: 'a', station: null, consoles: [] } });
    expect(s.players[0].consoles).toEqual([]);
  });
});

describe('apply phase transitions', () => {
  it('GameStarted flips phase to InProgress', () => {
    const s = new LobbyState();
    s.apply({ type: 'GameStarted' });
    expect(s.phase).toBe('InProgress');
  });

  it('GameOver flips phase and records the reason', () => {
    const s = new LobbyState();
    s.apply({ type: 'GameOver', data: { reason: 'Ship destroyed' } });
    expect(s.phase).toBe('GameOver');
    expect(s.gameOverReason).toBe('Ship destroyed');
  });
});

describe('ComplexityChanged is ignored (retired message)', () => {
  it('does not crash and does not mutate state', () => {
    const s = new LobbyState();
    const before = JSON.stringify(s);
    s.apply({ type: 'ComplexityChanged', data: { console: 'Helm', preset_name: 'Low' } });
    expect(JSON.stringify(s)).toBe(before);
  });
});

describe('apply ignores unrelated messages', () => {
  it('SimState does not disturb the lobby model', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', [])];
    const before = JSON.stringify(s);
    s.apply({ type: 'SimState', data: { snapshot: { red_alert: true, entity_states: [] } } });
    expect(JSON.stringify(s)).toBe(before);
  });
});

describe('view derivations', () => {
  it('isCaptain only for the CaptainChair holder', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', ['CaptainChair']), p('b', 'Bob', ['Helm'])];
    expect(s.isCaptain('a')).toBe(true);
    expect(s.isCaptain('b')).toBe(false);
    expect(s.isCaptain('ghost')).toBe(false);
    expect(s.isHelm('b')).toBe(true);
  });

  it('isSpectator true for unknown token or empty consoles', () => {
    const s = new LobbyState();
    s.players = [p('me', 'Me', [])];
    expect(s.isSpectator('me')).toBe(true);
    expect(s.isSpectator('ghost')).toBe(true);
  });

  it('showLobbyPanel: lobby always, in-progress only for spectators, game-over never', () => {
    const s = new LobbyState();
    s.players = [p('me', 'Me', ['Helm']), p('spec', 'Spec', [])];
    expect(s.showLobbyPanel('me')).toBe(true);
    s.phase = 'InProgress';
    expect(s.showLobbyPanel('me')).toBe(false);
    expect(s.showLobbyPanel('spec')).toBe(true);
    expect(s.gameInProgressBanner('spec')).toBe(true);
    s.phase = 'GameOver';
    expect(s.showLobbyPanel('spec')).toBe(false);
  });
});

describe('stationSlots', () => {
  it('shows one row per station from shipStations.stations', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [
      p('a', 'Alice', ['CaptainChair', 'Helm']),
      p('b', 'Bob', ['Tactical']),
    ];
    const names = s.stationSlots('x').filter(sl => sl.kind !== 'spectator').map(sl => sl.station);
    expect(names).toEqual(['Helm', 'Tactical']);
  });

  it('classifies mine / occupied / available', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [p('me', 'Me', ['CaptainChair', 'Helm']), p('b', 'Bob', ['Tactical'])];
    const slots = s.stationSlots('me');
    expect(slots[0].kind).toBe('mine');
    expect(slots[1].kind).toBe('occupied');
    expect(slots[1].holder_name).toBe('Bob');
  });

  it('available slot when a player has not yet picked a station', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [p('a', 'Alice', ['CaptainChair', 'Helm']), p('b', 'Bob', [])];
    const slots = s.stationSlots('x');
    // 2 station rows + 1 spectator (Bob has no station consoles)
    expect(slots).toHaveLength(3);
    expect(slots[0].kind).toBe('occupied');
    expect(slots[1].kind).toBe('available');
    expect(slots[2]).toEqual({ kind: 'spectator', player_name: 'Bob' });
  });

  it('appends spectator rows for players with no station consoles', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [
      p('a', 'Alice', ['CaptainChair', 'Helm']),
      p('b', 'Bob', ['Tactical']),
      p('c', 'Carol', []),
      p('d', 'Dave', []),
    ];
    const slots = s.stationSlots('x');
    // 2 station rows + 2 spectators
    expect(slots).toHaveLength(4);
    expect(slots[2]).toEqual({ kind: 'spectator', player_name: 'Carol' });
    expect(slots[3]).toEqual({ kind: 'spectator', player_name: 'Dave' });
  });

  it('shows all station rows even when no players are connected', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    const slots = s.stationSlots('x');
    expect(slots).toHaveLength(2);
    expect(slots[0].station).toBe('Helm');
    expect(slots[1].station).toBe('Tactical');
    expect(slots[0].kind).toBe('available');
  });

  it('slot shape contains station/short_code/description/rank/consoles but no preset_names', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    const slots = s.stationSlots('x');
    const slot = slots[0];
    expect(slot.station).toBe('Helm');
    expect(slot.short_code).toBe('HLM');
    expect(slot.description).toBe('Pilot');
    expect(slot.consoles).toEqual(['Helm', 'CaptainChair']);
    expect(slot).not.toHaveProperty('preset_names');
  });

  it('returns empty when shipStations has no stations', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', [])];
    const slots = s.stationSlots('x');
    // No station defs → no station rows. Alice has no station consoles → spectator.
    expect(slots).toEqual([{ kind: 'spectator', player_name: 'Alice' }]);
  });
});

describe('allStationsFilled', () => {
  it('true when every station console set is held', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [p('a', 'Alice', ['CaptainChair', 'Helm']), p('b', 'Bob', ['Tactical'])];
    expect(s.allStationsFilled()).toBe(true);
  });

  it('false when a station has no holder', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [p('a', 'Alice', ['CaptainChair', 'Helm']), p('b', 'Bob', [])];
    expect(s.allStationsFilled()).toBe(false);
  });

  it('false when shipStations.stations is empty', () => {
    const s = new LobbyState();
    s.players = [p('a', 'Alice', ['CaptainChair', 'Helm'])];
    expect(s.allStationsFilled()).toBe(false);
  });

  it('true even when overflow spectators exist', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [
      p('a', 'Alice', ['CaptainChair', 'Helm']),
      p('b', 'Bob', ['Tactical']),
      p('c', 'Carol', []),
    ];
    expect(s.allStationsFilled()).toBe(true);
  });
});

describe('reconcileActiveConsole', () => {
  it('keeps current when present in the new bundle', () => {
    expect(reconcileActiveConsole('Helm', ['CaptainChair', 'Helm'])).toBe('Helm');
  });
  it('jumps to first when current not in bundle', () => {
    expect(reconcileActiveConsole('Sensors', ['CaptainChair', 'Helm'])).toBe('CaptainChair');
  });
  it('null current lands on first console', () => {
    expect(reconcileActiveConsole(null, ['Tactical'])).toBe('Tactical');
  });
  it('returns null for an empty bundle (spectator)', () => {
    expect(reconcileActiveConsole('Helm', [])).toBeNull();
  });
});

describe('consolesOf', () => {
  it('passes through a consoles array and normalises legacy single console', () => {
    expect(consolesOf({ consoles: ['Helm'] })).toEqual(['Helm']);
    expect(consolesOf({ console: 'Helm' })).toEqual(['Helm']);
    expect(consolesOf({})).toEqual([]);
  });
});
