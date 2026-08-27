import { describe, it, expect } from 'vitest';
import {
  LobbyState, lobbyState, reconcileActiveConsole, nextActiveConsole,
  ALL_STATIONS, playerStationId,
} from '../../gui/lobby-state.js';
import { CHANGE_DOMAINS } from '../../gui/reducer-result.js';

// Post issue #619: a player carries a single lowercase station id (or null).
function ps(token, name, station) {
  return { token, name, station, connected: true };
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

// Post-#619 station def: no consoles list, just id/name/description/rank/short_code.
const TWO_STATION_SHIP = {
  stations: [
    { id: 'helm', name: 'Helm', description: 'Pilot', rank: '', short_code: 'HLM' },
    { id: 'tactical', name: 'Tactical', description: 'Weapons', rank: '', short_code: 'TAC' },
  ],
};

describe('semantic reducer results', () => {
  const cases = [
    { type: 'Welcome' },
    { type: 'ScenarioCatalog' },
    { type: 'PlayerJoined', data: { player: ps('b', 'Bob', null) } },
    { type: 'PlayerLeft', data: { token: 'a' } },
    { type: 'NameChanged', data: { token: 'a', name: 'Alicia' } },
    { type: 'ReadyChanged', data: { token: 'a', ready: true } },
    { type: 'SpectatorChanged', data: { token: 'a', spectator: true } },
    { type: 'AfkChanged', data: { token: 'a', afk: true } },
    { type: 'StationAssigned', data: { token: 'a', station_id: 'helm' } },
    { type: 'GameStartCountdown', data: { remaining_secs: 3 } },
    { type: 'GameStarted' },
    { type: 'GameOver' },
    { type: 'ReturnedToLobby' },
  ];

  for (const msg of cases) {
    it(`${msg.type} reports an accepted lobby change`, () => {
      const state = new LobbyState();
      state.players = [ps('a', 'Alice', 'captain')];
      const changes = state.apply(msg);
      expect(changes.changedDomains).toContain(CHANGE_DOMAINS.LOBBY);
    });
  }

  it('ignored or inapplicable messages return no semantic change', () => {
    const state = new LobbyState();
    expect([...state.apply({ type: 'NoSuchMessage' }).changedDomains]).toEqual([]);
    expect([...state.apply({
      type: 'NameChanged', data: { token: 'missing', name: 'Nobody' },
    }).changedDomains]).toEqual([]);
  });
});

describe('LobbyState defaults', () => {
  it('starts empty in Lobby phase', () => {
    const s = new LobbyState();
    expect(s.phase).toBe('Lobby');
    expect(s.players).toEqual([]);
    expect(s.gameOverReason).toBeNull();
  });

  it('exports a singleton and the canonical station list', () => {
    expect(lobbyState).toBeInstanceOf(LobbyState);
    expect(ALL_STATIONS).toHaveLength(10);
    expect(ALL_STATIONS[0]).toBe('captain');
  });
});

describe('apply Welcome', () => {
  it('replaces state wholesale', () => {
    const s = new LobbyState();
    s.players.push(ps('ghost', 'Ghost', null));
    s.apply(welcome({
      phase: 'Lobby',
      players: [ps('a', 'Alice', 'captain')],
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

  it('normalises player.station into a lowercase station id string', () => {
    const s = new LobbyState();
    s.apply(welcome({
      phase: 'Lobby',
      players: [ps('a', 'Alice', 'helm')],
      world: null,
    }, TWO_STATION_SHIP));
    expect(s.players[0].station).toBe('helm');
  });
});

describe('apply player lifecycle', () => {
  it('PlayerJoined appends new players in order', () => {
    const s = new LobbyState();
    s.apply({ type: 'PlayerJoined', data: { player: ps('a', 'Alice', null) } });
    s.apply({ type: 'PlayerJoined', data: { player: ps('b', 'Bob', null) } });
    expect(s.players.map(x => x.token)).toEqual(['a', 'b']);
  });

  it('PlayerJoined with existing token replaces in place', () => {
    const s = new LobbyState();
    s.apply({ type: 'PlayerJoined', data: { player: ps('a', 'Alice', 'helm') } });
    s.apply({ type: 'PlayerJoined', data: { player: ps('a', 'Alice2', null) } });
    expect(s.players).toHaveLength(1);
    expect(s.players[0].name).toBe('Alice2');
    expect(s.players[0].station).toBeNull();
  });

  it('PlayerJoined records the lowercase station id on the player', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.apply({ type: 'PlayerJoined', data: { player: ps('a', 'Alice', 'tactical') } });
    expect(s.players[0].station).toBe('tactical');
  });

  it('PlayerLeft removes by token', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', null), ps('b', 'Bob', null)];
    s.apply({ type: 'PlayerLeft', data: { token: 'a' } });
    expect(s.players.map(x => x.token)).toEqual(['b']);
  });

  it('NameChanged updates only the named player', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', null), ps('b', 'Bob', null)];
    s.apply({ type: 'NameChanged', data: { token: 'a', name: 'Alicia' } });
    expect(s.players[0].name).toBe('Alicia');
    expect(s.players[1].name).toBe('Bob');
  });

  it('normalises a new player with spectator=false by default (issue #1105)', () => {
    const s = new LobbyState();
    s.apply({ type: 'PlayerJoined', data: { player: ps('a', 'Alice', null) } });
    expect(s.players[0].spectator).toBe(false);
  });

  it('SpectatorChanged toggles the role flag on the named player (issue #1105)', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', null), ps('b', 'Bob', null)];
    s.apply({ type: 'SpectatorChanged', data: { token: 'a', spectator: true } });
    expect(s.players[0].spectator).toBe(true);
    expect(s.players[1].spectator).toBeFalsy();
    s.apply({ type: 'SpectatorChanged', data: { token: 'a', spectator: false } });
    expect(s.players[0].spectator).toBe(false);
  });

  it('AfkChanged toggles the presence flag without touching the seat (issue #1104)', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', 'helm'), ps('b', 'Bob', null)];
    s.apply({ type: 'AfkChanged', data: { token: 'a', afk: true } });
    expect(s.players[0].afk).toBe(true);
    expect(s.players[0].station).toBe('helm'); // AFK retains the seat
    expect(s.players[1].afk).toBeFalsy();
    s.apply({ type: 'AfkChanged', data: { token: 'a', afk: false } });
    expect(s.players[0].afk).toBe(false);
    expect(s.players[0].station).toBe('helm');
  });

  it('returning from AFK does not steal console focus (issue #1104 AC4)', () => {
    // AFK never changes Player.station, so no StationAssigned is re-sent and the
    // reconciler keeps the current console across the enter/leave round-trip.
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', 'helm')];
    const held = () => s.players.filter((p) => p.station).map((p) => p.station);
    expect(reconcileActiveConsole('helm', held())).toBe('helm');
    s.apply({ type: 'AfkChanged', data: { token: 'a', afk: true } });
    s.apply({ type: 'AfkChanged', data: { token: 'a', afk: false } });
    expect(reconcileActiveConsole('helm', held())).toBe('helm');
  });
});

describe('apply StationAssigned', () => {
  it('assigns the station id to the named player', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', null), ps('b', 'Bob', null)];
    s.apply({ type: 'StationAssigned', data: { token: 'a', station: 'Captain', station_id: 'captain' } });
    expect(s.players[0].station).toBe('captain');
    expect(s.players[1].station).toBeNull();
  });

  it('steals the station from the previous holder first', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', 'helm'), ps('b', 'Bob', null)];
    s.apply({ type: 'StationAssigned', data: { token: 'b', station: 'Helm', station_id: 'helm' } });
    expect(s.players[0].station).toBeNull();
    expect(s.players[1].station).toBe('helm');
  });

  it('spectator assignment (no station_id) clears the player station', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', 'captain')];
    s.apply({ type: 'StationAssigned', data: { token: 'a', station: null, station_id: null } });
    expect(s.players[0].station).toBeNull();
  });

  it('seating a spectator clears the local spectator flag (issue #1106 fix A)', () => {
    // Mirrors the Rust set_station invariant: on a claim the host emits
    // StationAssigned (+ RatingChanged) but NO SpectatorChanged, so the client
    // roster must drop the spectator role itself or the claimant stays stuck on
    // the read-only spectator surface.
    const s = new LobbyState();
    s.players = [{ ...ps('a', 'Alice', null), spectator: true }];
    s.apply({ type: 'StationAssigned', data: { token: 'a', station: 'Helm', station_id: 'helm' } });
    expect(s.players[0].station).toBe('helm');
    expect(s.players[0].spectator).toBe(false);
  });

  it('a station-vacate (null station_id) does not touch the spectator flag', () => {
    // Only seating clears the role; releasing a seat must not silently flip a
    // player into (or out of) the Spectator role — that arrives via SpectatorChanged.
    // Start from spectator:true so this also guards against a clear-on-vacate
    // regression (the fix-A clear must be gated on a non-null station_id only).
    const s = new LobbyState();
    s.players = [{ ...ps('a', 'Alice', 'captain'), spectator: true }];
    s.apply({ type: 'StationAssigned', data: { token: 'a', station: null, station_id: null } });
    expect(s.players[0].station).toBeNull();
    expect(s.players[0].spectator).toBe(true);
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

  it('ReturnedToLobby sets phase to Lobby and waitingForScenario to true', () => {
    const s = new LobbyState();
    s.phase = 'GameOver';
    s.gameOverReason = 'All consoles destroyed';
    s.apply({ type: 'ReturnedToLobby' });
    expect(s.phase).toBe('Lobby');
    expect(s.gameOverReason).toBeNull();
    expect(s.waitingForScenario).toBe(true);
  });

  it('Welcome clears waitingForScenario via replaceFrom', () => {
    const s = new LobbyState();
    s.waitingForScenario = true;
    s.apply({
      type: 'Welcome',
      data: {
        state: { phase: 'Lobby', players: [], world: null },
        ship_stations: { stations: [] },
        ship_config: {},
      },
    });
    expect(s.waitingForScenario).toBe(false);
  });
});

describe('ScenarioCatalog (QR-first pre-scenario picker, issue #755)', () => {
  const catalogMsg = (locked_scenario = null, locked_ship = null, active_packs = []) => ({
    type: 'ScenarioCatalog',
    data: {
      scenarios: [
        {
          id: 'default',
          world: 'assets/worlds/default.toml',
          label: 'Starbase Alpha',
          source: 'base',
          ships: [{ template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser' }],
        },
      ],
      locked_scenario,
      locked_ship,
      active_packs,
    },
  });

  it('defaults to no picker (scenarioCatalog null)', () => {
    const s = new LobbyState();
    expect(s.scenarioCatalog).toBeNull();
    expect(s.showScenarioPicker()).toBe(false);
  });

  it('applies the catalog and lock state, enabling the picker', () => {
    const s = new LobbyState();
    s.apply(catalogMsg());
    expect(s.showScenarioPicker()).toBe(true);
    expect(s.scenarioCatalog).toHaveLength(1);
    expect(s.scenarioCatalog[0].id).toBe('default');
    expect(s.selectionLocked).toEqual({ scenario_id: null, template_path: null });
  });

  it('reflects the locked selection from the host', () => {
    const s = new LobbyState();
    s.apply(catalogMsg('default', 'assets/entities/alliance_cruiser.toml'));
    expect(s.selectionLocked).toEqual({
      scenario_id: 'default',
      template_path: 'assets/entities/alliance_cruiser.toml',
    });
  });

  it('Welcome clears the picker once the world has loaded', () => {
    const s = new LobbyState();
    s.apply(catalogMsg());
    expect(s.showScenarioPicker()).toBe(true);
    s.apply({
      type: 'Welcome',
      data: {
        state: { phase: 'Lobby', players: [], world: null },
        ship_stations: { stations: [] },
        ship_config: {},
      },
    });
    expect(s.scenarioCatalog).toBeNull();
    expect(s.showScenarioPicker()).toBe(false);
  });

  // Second round after Game Over (issue #756): ReturnedToLobby re-shows the
  // picker via a re-broadcast catalog, then a fully-locked catalog is the
  // "selection done" signal that resumes the lobby (replacing ScenarioLoaded)
  // for phones reusing the already-loaded world — no fresh Welcome arrives.
  it('second round: ReturnedToLobby then re-broadcast catalog re-renders the picker', () => {
    const s = new LobbyState();
    s.phase = 'GameOver';
    s.apply({ type: 'ReturnedToLobby' });
    expect(s.waitingForScenario).toBe(true);
    expect(s.showScenarioPicker()).toBe(false);
    // Host re-broadcasts the (unlocked) catalog after the return.
    s.apply(catalogMsg());
    expect(s.showScenarioPicker()).toBe(true);
    expect(s.scenarioCatalog).toHaveLength(1);
  });

  it('second round: a fully-locked catalog resumes the lobby (no Welcome needed)', () => {
    const s = new LobbyState();
    s.phase = 'GameOver';
    s.apply({ type: 'ReturnedToLobby' });
    s.apply(catalogMsg());
    expect(s.showScenarioPicker()).toBe(true);
    // driveWorldLoad broadcasts the completed lock → phone leaves the picker
    // and the waiting overlay, resuming the lobby to re-crew for the round.
    s.apply(catalogMsg('default', 'assets/entities/alliance_cruiser.toml'));
    expect(s.scenarioCatalog).toBeNull();
    expect(s.showScenarioPicker()).toBe(false);
    expect(s.waitingForScenario).toBe(false);
    expect(s.phase).toBe('Lobby');
  });
});

describe('active mod packs (issue #990)', () => {
  const PACKS = [
    { id: 'aurora-skirmish', name: 'Aurora Skirmish', version: '1.0.0' },
    { id: 'nebula-run', name: 'Nebula Run', version: '2.1' },
  ];
  const catalogMsg = (active_packs = [], locked_scenario = null, locked_ship = null) => ({
    type: 'ScenarioCatalog',
    data: {
      scenarios: [{ id: 'default', world: 'assets/worlds/default.toml', source: 'base', ships: [] }],
      locked_scenario,
      locked_ship,
      active_packs,
    },
  });
  const welcomeMsg = () => ({
    type: 'Welcome',
    data: {
      state: { phase: 'Lobby', players: [], world: null },
      ship_stations: { stations: [] },
      ship_config: {},
    },
  });

  it('defaults to an empty active-pack list', () => {
    expect(new LobbyState().activePacks).toEqual([]);
  });

  it('stores the active packs from a ScenarioCatalog message', () => {
    const s = new LobbyState();
    s.apply(catalogMsg(PACKS));
    expect(s.activePacks).toEqual(PACKS);
  });

  it('treats a catalog with no active_packs as no packs applied', () => {
    const s = new LobbyState();
    s.apply({ type: 'ScenarioCatalog', data: { scenarios: [], locked_scenario: null, locked_ship: null } });
    expect(s.activePacks).toEqual([]);
  });

  it('SURVIVES world load — Welcome does not clear it (issue #990 AC5)', () => {
    const s = new LobbyState();
    s.apply(catalogMsg(PACKS));
    s.apply(welcomeMsg());
    // The picker is gone, but the mods list persists — a mid-round player must
    // still see what they are playing.
    expect(s.scenarioCatalog).toBeNull();
    expect(s.activePacks).toEqual(PACKS);
  });

  it('the locked-catalog "selection done" broadcast still carries the packs', () => {
    const s = new LobbyState();
    // Fully-locked catalog (scenario + ship): scenarioCatalog is cleared, but
    // active_packs on the same message keeps the list alive.
    s.apply(catalogMsg(PACKS, 'default', 'assets/entities/alliance_cruiser.toml'));
    expect(s.scenarioCatalog).toBeNull();
    expect(s.activePacks).toEqual(PACKS);
  });

  it('reset() clears the active packs (the same wipe that clears the catalog)', () => {
    const s = new LobbyState();
    s.apply(catalogMsg(PACKS));
    s.reset();
    expect(s.activePacks).toEqual([]);
    expect(s.scenarioCatalog).toBeNull();
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
    s.players = [ps('a', 'Alice', null)];
    const before = JSON.stringify(s);
    s.apply({ type: 'SimState', data: { snapshot: { red_alert: true, entity_states: [] } } });
    expect(JSON.stringify(s)).toBe(before);
  });
});

describe('view derivations', () => {
  it('isCaptain only for the captain station holder', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', 'captain'), ps('b', 'Bob', 'helm')];
    expect(s.isCaptain('a')).toBe(true);
    expect(s.isCaptain('b')).toBe(false);
    expect(s.isCaptain('ghost')).toBe(false);
    expect(s.isHelm('b')).toBe(true);
  });

  it('playerStation returns the lowercase station id', () => {
    const s = new LobbyState();
    s.players = [ps('me', 'Me', 'helm')];
    expect(s.playerStation('me')).toBe('helm');
    expect(s.isHelm('me')).toBe(true);
    expect(s.isCaptain('me')).toBe(false);
  });

  it('isSpectator true for unknown token or no station', () => {
    const s = new LobbyState();
    s.players = [ps('me', 'Me', null)];
    expect(s.isSpectator('me')).toBe(true);
    expect(s.isSpectator('ghost')).toBe(true);
  });

  it('showLobbyPanel: lobby always, in-progress for spectators and unready claimants, game-over never', () => {
    const s = new LobbyState();
    s.players = [
      { ...ps('me', 'Me', 'helm'), ready: true },
      { ...ps('pending', 'Pending', 'tactical'), ready: false },
      ps('spec', 'Spec', null),
    ];
    expect(s.showLobbyPanel('me')).toBe(true);
    s.phase = 'InProgress';
    expect(s.showLobbyPanel('me')).toBe(false);
    expect(s.showLobbyPanel('pending')).toBe(true);
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
      ps('a', 'Alice', 'helm'),
      ps('b', 'Bob', 'tactical'),
    ];
    const names = s.stationSlots('x').filter(sl => sl.kind !== 'spectator').map(sl => sl.station);
    expect(names).toEqual(['Helm', 'Tactical']);
  });

  it('classifies mine / occupied / available', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [ps('me', 'Me', 'helm'), ps('b', 'Bob', 'tactical')];
    const slots = s.stationSlots('me');
    expect(slots[0].kind).toBe('mine');
    expect(slots[1].kind).toBe('occupied');
    expect(slots[1].holder_name).toBe('Bob');
  });

  it('classifies station-based players', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [ps('me', 'Me', 'helm'), ps('b', 'Bob', 'tactical')];
    const slots = s.stationSlots('me');
    expect(slots[0].kind).toBe('mine');
    expect(slots[1].kind).toBe('occupied');
    expect(slots[1].holder_name).toBe('Bob');
  });

  it('available slot when a player has not yet picked a station', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [ps('a', 'Alice', 'helm'), ps('b', 'Bob', null)];
    const slots = s.stationSlots('x');
    // 2 station rows + 1 spectator (Bob has no station)
    expect(slots).toHaveLength(3);
    expect(slots[0].kind).toBe('occupied');
    expect(slots[1].kind).toBe('available');
    expect(slots[2]).toEqual({ kind: 'spectator', player_name: 'Bob' });
  });

  it('appends spectator rows for players with no station', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [
      ps('a', 'Alice', 'helm'),
      ps('b', 'Bob', 'tactical'),
      ps('c', 'Carol', null),
      ps('d', 'Dave', null),
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

  it('slot shape contains station/short_code/description/rank but no consoles or preset_names', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    const slots = s.stationSlots('x');
    const slot = slots[0];
    expect(slot.station).toBe('Helm');
    expect(slot.short_code).toBe('HLM');
    expect(slot.description).toBe('Pilot');
    expect(slot).not.toHaveProperty('consoles');
    expect(slot).not.toHaveProperty('preset_names');
  });

  it('returns empty when shipStations has no stations', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', null)];
    const slots = s.stationSlots('x');
    // No station defs → no station rows. Alice has no station → spectator.
    expect(slots).toEqual([{ kind: 'spectator', player_name: 'Alice' }]);
  });
});

describe('allStationsFilled', () => {
  it('true when every station is held', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [ps('a', 'Alice', 'helm'), ps('b', 'Bob', 'tactical')];
    expect(s.allStationsFilled()).toBe(true);
  });

  it('false when a station has no holder', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [ps('a', 'Alice', 'helm'), ps('b', 'Bob', null)];
    expect(s.allStationsFilled()).toBe(false);
  });

  it('false when shipStations.stations is empty', () => {
    const s = new LobbyState();
    s.players = [ps('a', 'Alice', 'helm')];
    expect(s.allStationsFilled()).toBe(false);
  });

  it('true even when overflow spectators exist', () => {
    const s = new LobbyState();
    s.shipStations = TWO_STATION_SHIP;
    s.players = [
      ps('a', 'Alice', 'helm'),
      ps('b', 'Bob', 'tactical'),
      ps('c', 'Carol', null),
    ];
    expect(s.allStationsFilled()).toBe(true);
  });
});

describe('reconcileActiveConsole', () => {
  it('keeps current when present in the new list', () => {
    expect(reconcileActiveConsole('helm', ['captain', 'helm'])).toBe('helm');
  });
  it('jumps to first when current not in list', () => {
    expect(reconcileActiveConsole('sensors', ['captain', 'helm'])).toBe('captain');
  });
  it('null current lands on first station id', () => {
    expect(reconcileActiveConsole(null, ['tactical'])).toBe('tactical');
  });
  it('returns null for an empty list (spectator)', () => {
    expect(reconcileActiveConsole('helm', [])).toBeNull();
  });
});

describe('playerStationId', () => {
  it('reads a lowercase station id from a string field', () => {
    expect(playerStationId({ station: 'helm' })).toBe('helm');
  });
  it('reads a nested object shape { id }', () => {
    expect(playerStationId({ station: { id: 'tactical' } })).toBe('tactical');
  });
  it('returns null when no station is set', () => {
    expect(playerStationId({})).toBeNull();
    expect(playerStationId(null)).toBeNull();
  });
});

// Ported from the deleted tests/client/active-console.test.js (issue #827):
// nextActiveConsole moved here when gui/active-console.js was absorbed.
describe('nextActiveConsole', () => {
  it('normalises null to null', () => {
    expect(nextActiveConsole('captain', null))
      .toEqual({ changed: true, next: null });
  });

  it('normalises undefined to null', () => {
    expect(nextActiveConsole('tactical', undefined))
      .toEqual({ changed: true, next: null });
  });

  it('normalises empty string to null', () => {
    expect(nextActiveConsole('helm', ''))
      .toEqual({ changed: true, next: null });
  });

  it('no change when both are null', () => {
    expect(nextActiveConsole(null, null).changed).toBe(false);
  });

  it('no change when the name matches the current console', () => {
    expect(nextActiveConsole('helm', 'helm').changed).toBe(false);
  });

  it('switches between consoles', () => {
    expect(nextActiveConsole('helm', 'tactical'))
      .toEqual({ changed: true, next: 'tactical' });
  });

  it('activates from null', () => {
    expect(nextActiveConsole(null, 'captain'))
      .toEqual({ changed: true, next: 'captain' });
  });

  it('activates from undefined current', () => {
    expect(nextActiveConsole(undefined, 'helm'))
      .toEqual({ changed: true, next: 'helm' });
  });

  it('empty-string current equals null current', () => {
    expect(nextActiveConsole('', null).changed).toBe(false);
  });
});
