import { describe, it, expect } from 'vitest';
import {
  routeMessage,
  showingLobbyDuringGame,
  isSpectator,
  playerStation,
} from '../../gui/client-router.js';

const MY = 'tok-me';

function baseUiState(overrides = {}) {
  return {
    players: [],
    phase: 'Lobby',
    shipStations: { stations: [] },
    stations: [],
    maxPlayers: 0,
    allFilled: false,
    allReady: false,
    countdownSecs: 0,
    shipDestroyed: false,
    gameOverReason: null,
    ...overrides,
  };
}

function ctx(uiState, overrides = {}) {
  return {
    uiState,
    myToken: MY,
    pendingMidGameClaim: false,
    bezelAlertOn: false,
    lobbyState: null,
    redAlert: false,
    ...overrides,
  };
}

const effects = (r) => r.sideEffects.map(fx => fx.effect);

describe('showingLobbyDuringGame', () => {
  it('is false outside InProgress', () => {
    expect(showingLobbyDuringGame(baseUiState(), MY, false)).toBe(false);
    expect(showingLobbyDuringGame(baseUiState({ phase: 'GameOver' }), MY, false)).toBe(false);
  });

  it('is true for a spectator (no station) during InProgress', () => {
    const s = baseUiState({ phase: 'InProgress', players: [] });
    expect(showingLobbyDuringGame(s, MY, false)).toBe(true);
  });

  it('is true while a mid-game claim is pending', () => {
    const s = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: true }],
    });
    expect(showingLobbyDuringGame(s, MY, true)).toBe(true);
  });

  it('is true when the player holds a station but has not pressed Ready', () => {
    const s = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: false }],
    });
    expect(showingLobbyDuringGame(s, MY, false)).toBe(true);
  });

  it('is false for a seated, ready player', () => {
    const s = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: true }],
    });
    expect(showingLobbyDuringGame(s, MY, false)).toBe(false);
  });

  it('is false for an explicit Spectator (issue #1105) — they get the summary surface', () => {
    const s = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: null, ready: false, spectator: true }],
    });
    expect(showingLobbyDuringGame(s, MY, false)).toBe(false);
  });
});

describe('isSpectator', () => {
  it('is true only for an explicit spectator flag', () => {
    const spec = baseUiState({ players: [{ token: MY, spectator: true }] });
    const stationless = baseUiState({ players: [{ token: MY, station: null }] });
    expect(isSpectator(spec, MY)).toBe(true);
    expect(isSpectator(stationless, MY)).toBe(false);
    expect(isSpectator(baseUiState(), MY)).toBe(false); // unknown token
  });
});

describe('routeMessage — Welcome', () => {
  const welcome = {
    type: 'Welcome',
    data: {
      state: {
        phase: 'Lobby',
        players: [{ token: MY, name: 'Ada', ready: false, station: null }],
        world: { scenario_title: 'Falling Skyway', scenario_description: 'Hold the line' },
      },
      ship_stations: { stations: [{ id: 'helm', console: 'gui/battleship/helm.html' }] },
      ship_config: { ship_css: 'gui/battleship/theme.css' },
    },
  };

  it('applies the msg fallback mutations when lobbyState is absent', () => {
    const s = baseUiState();
    const r = routeMessage(welcome, ctx(s));
    expect(s.phase).toBe('Lobby');
    expect(s.shipStations.stations).toHaveLength(1);
    expect(s.players).toHaveLength(1);
    expect(r.rebuildStations).toBe(true);
  });

  it('emits mount-consoles, ship-theme, ship-info and status in order', () => {
    const s = baseUiState();
    const r = routeMessage(welcome, ctx(s));
    expect(effects(r)).toEqual(['mount-consoles', 'ship-theme', 'ship-info', 'status']);
    expect(r.sideEffects[0].shipStations).toBe(s.shipStations);
    expect(r.sideEffects[1].href).toBe('gui/battleship/theme.css');
    expect(r.sideEffects[2]).toEqual({
      effect: 'ship-info', title: 'Falling Skyway', description: 'Hold the line',
    });
    expect(r.sideEffects[3].id).toBe('client.status_connected');
  });

  it('omits ship-theme / ship-info when absent, and prefers the lobbyState mirror', () => {
    const s = baseUiState();
    const mirror = {
      phase: 'Lobby',
      players: [{ token: 'x', name: 'Mirror' }],
      shipStations: { stations: [] },
    };
    const msg = { type: 'Welcome', data: { state: { phase: 'Lobby', players: [] } } };
    const r = routeMessage(msg, ctx(s, { lobbyState: mirror }));
    expect(s.players).toBe(mirror.players);
    expect(effects(r)).toEqual(['mount-consoles', 'status']);
  });
});

describe('routeMessage — StationAssigned', () => {
  it('clears any other holder of the same station (msg fallback path)', () => {
    const s = baseUiState({
      players: [
        { token: 'other', name: 'Bob', station: 'helm', ready: true },
        { token: MY, name: 'Ada', station: null, ready: false },
      ],
    });
    const msg = { type: 'StationAssigned', data: { token: MY, station_id: 'helm' } };
    const r = routeMessage(msg, ctx(s));
    expect(s.players.find(p => p.token === 'other').station).toBeNull();
    expect(s.players.find(p => p.token === MY).station).toBe('helm');
    expect(r.rebuildStations).toBe(true);
  });

  it('accepts an object station_id { id }', () => {
    const s = baseUiState({ players: [{ token: MY, station: null }] });
    const msg = { type: 'StationAssigned', data: { token: MY, station_id: { id: 'captain' } } };
    routeMessage(msg, ctx(s));
    expect(playerStation(s, MY)).toBe('captain');
  });

  it('clears pendingMidGameClaim when my assignment is severed (null station)', () => {
    const s = baseUiState({ players: [{ token: MY, station: 'helm' }] });
    const msg = { type: 'StationAssigned', data: { token: MY, station_id: null } };
    const r = routeMessage(msg, ctx(s, { pendingMidGameClaim: true }));
    expect(r.pendingMidGameClaim).toBe(false);
  });

  it('keeps pendingMidGameClaim on someone ELSE being severed', () => {
    const s = baseUiState({ players: [{ token: 'other', station: 'helm' }] });
    const msg = { type: 'StationAssigned', data: { token: 'other', station_id: null } };
    const r = routeMessage(msg, ctx(s, { pendingMidGameClaim: true }));
    expect(r.pendingMidGameClaim).toBe(true);
  });
});

describe('routeMessage — SimState render guard', () => {
  it('skips the render entirely while showing the lobby over a game (spectator)', () => {
    const s = baseUiState({ phase: 'InProgress', players: [] });
    const r = routeMessage({ type: 'SimState', data: {} }, ctx(s));
    expect(r.shouldRender).toBe(false);
    expect(r.rebuildStations).toBe(false);
    expect(r.sideEffects).toEqual([]);
  });

  it('renders for a seated, ready player', () => {
    const s = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: true }],
    });
    const r = routeMessage({ type: 'SimState', data: {} }, ctx(s));
    expect(r.shouldRender).toBe(true);
  });
});

// Issue #1099 AC1: console iframes are (re)mounted ONLY on the Welcome
// `mount-consoles` side effect. A hosting change (SimState.station_hosts) or a
// station reassignment must NOT re-mount, or a visiting Station's persistent
// iframe — and its session-local interface context — would be torn down under
// the player. The iframe-node persistence itself is pinned in
// console-persistence.test.js; this pins the router seam that must never ask
// for a remount outside Welcome.
describe('routeMessage — consoles re-mount only on Welcome (issue #1099 AC1)', () => {
  const seated = () => baseUiState({
    phase: 'InProgress',
    players: [{ token: MY, name: 'Ada', station: 'helm', ready: true }],
  });

  it('a SimState hosting update emits no mount-consoles side effect', () => {
    const msg = {
      type: 'SimState',
      data: { snapshot: { station_hosts: [{ station: 'comms', host: 'helm', rating: 'Std' }] } },
    };
    const r = routeMessage(msg, ctx(seated()));
    expect(effects(r)).not.toContain('mount-consoles');
    expect(effects(r)).toEqual([]);
  });

  it('non-Welcome lifecycle messages never re-mount consoles', () => {
    for (const msg of [
      { type: 'StationAssigned', data: { token: MY, station_id: 'comms' } },
      { type: 'RatingChanged', data: { station_id: 'comms', rating_name: 'Std' } },
      { type: 'BlackboardUpdate', data: { updates: [['comms', {}]] } },
      { type: 'PlayerJoined', data: { player: { token: 'x', name: 'X' } } },
    ]) {
      const r = routeMessage(msg, ctx(seated()));
      expect(effects(r)).not.toContain('mount-consoles');
    }
  });
});

describe('routeMessage — BlackboardUpdate', () => {
  const bbMsg = (systems) => ({
    type: 'BlackboardUpdate',
    data: { updates: systems.map(id => [id, {}]) },
  });
  const seated = () => baseUiState({
    phase: 'InProgress',
    players: [{ token: MY, station: 'helm', ready: true }],
  });

  it('emits bezel-alert only when the captain/viewscreen alert flag CHANGES', () => {
    const r1 = routeMessage(bbMsg(['captain']), ctx(seated(), { redAlert: true, bezelAlertOn: false }));
    expect(effects(r1)).toEqual(['bezel-alert']);
    expect(r1.sideEffects[0].on).toBe(true);
    expect(r1.bezelAlertOn).toBe(true);

    const r2 = routeMessage(bbMsg(['captain']), ctx(seated(), { redAlert: true, bezelAlertOn: true }));
    expect(r2.sideEffects).toEqual([]);
    expect(r2.bezelAlertOn).toBe(true);
  });

  it('ignores the bezel for non-captain system updates', () => {
    const r = routeMessage(bbMsg(['helm-throttle']), ctx(seated(), { redAlert: true }));
    expect(r.sideEffects).toEqual([]);
    expect(r.bezelAlertOn).toBe(false);
  });

  it('applies the bezel side effect but SKIPS the render while lobby-over-game', () => {
    const spect = baseUiState({ phase: 'InProgress', players: [] });
    const r = routeMessage(bbMsg(['viewscreen']), ctx(spect, { redAlert: true }));
    expect(effects(r)).toEqual(['bezel-alert']);
    expect(r.shouldRender).toBe(false);
  });

  it('renders for a seated player', () => {
    const r = routeMessage(bbMsg(['captain']), ctx(seated()));
    expect(r.shouldRender).toBe(true);
  });
});

describe('routeMessage — GameOver / lifecycle', () => {
  it('GameOver sets the phase and reason and renders', () => {
    const s = baseUiState({ phase: 'InProgress' });
    const r = routeMessage({ type: 'GameOver', data: { reason: 'Mission failed' } }, ctx(s));
    expect(s.phase).toBe('GameOver');
    expect(s.gameOverReason).toBe('Mission failed');
    expect(r.shouldRender).toBe(true);
  });

  it('ShipDestroyed sets the flag', () => {
    const s = baseUiState();
    routeMessage({ type: 'ShipDestroyed', data: {} }, ctx(s));
    expect(s.shipDestroyed).toBe(true);
  });

  it('GameStarted flips the phase and hides the loading overlay', () => {
    const s = baseUiState({ countdownSecs: 3 });
    const r = routeMessage({ type: 'GameStarted', data: {} }, ctx(s));
    expect(s.phase).toBe('InProgress');
    expect(s.countdownSecs).toBe(0);
    expect(effects(r)).toEqual(['hide-loading']);
  });

  it('LoadingProgress emits show-loading with a rounded pct', () => {
    const r = routeMessage({ type: 'LoadingProgress', data: { fraction: 0.427 } }, ctx(baseUiState()));
    expect(r.sideEffects).toEqual([{ effect: 'show-loading', pct: 43 }]);
  });
});

describe('routeMessage — no-render cases', () => {
  it('DamageTaken vibrates scaled by hull damage and never renders', () => {
    const r = routeMessage({ type: 'DamageTaken', data: { hull: 30 } }, ctx(baseUiState()));
    expect(r.sideEffects).toEqual([{ effect: 'vibrate', duration: 300 }]);
    expect(r.shouldRender).toBe(false);

    const r0 = routeMessage({ type: 'DamageTaken', data: { hull: 0 } }, ctx(baseUiState()));
    expect(r0.sideEffects).toEqual([]);
  });

  it('CoordinationPopup emits the popup effect and never renders', () => {
    const address = { type: 'Station', data: 'tactical' };
    const presentation = { title: 'Alert', body: 'Brace' };
    const msg = {
      type: 'CoordinationPopup',
      data: {
        address, payload: { type: 'Alert' }, presentation,
        sender_label: 'Helm AI', to_label: 'Tactical',
      },
    };
    const r = routeMessage(msg, ctx(baseUiState()));
    expect(r.sideEffects).toEqual([{
      effect: 'coordination-popup', address, presentation,
      senderLabel: 'Helm AI', targetLabel: 'Tactical',
    }]);
    expect(r.shouldRender).toBe(false);
  });

  it('ship-wide CoordinationPopup keeps its typed address and authoritative Ship label', () => {
    const address = { type: 'Ship' };
    const presentation = { title: 'Standing down', body: '' };
    const r = routeMessage({
      type: 'CoordinationPopup',
      data: {
        address, payload: { type: 'IntentAdvisory' }, presentation,
        sender_label: 'Tactical', to_label: 'Ship',
      },
    }, ctx(baseUiState()));
    expect(r.sideEffects[0]).toMatchObject({
      effect: 'coordination-popup', address, presentation, targetLabel: 'Ship',
    });
  });

  it('WorldSetup / EntitySpawned / AsteroidSpawned skip the render', () => {
    for (const type of ['WorldSetup', 'EntitySpawned', 'AsteroidSpawned']) {
      expect(routeMessage({ type, data: {} }, ctx(baseUiState())).shouldRender).toBe(false);
    }
  });

  it('unknown message types skip the render', () => {
    const r = routeMessage({ type: 'Bogus', data: {} }, ctx(baseUiState()));
    expect(r.shouldRender).toBe(false);
    expect(r.sideEffects).toEqual([]);
  });
});

describe('routeMessage — player list bookkeeping', () => {
  it('ReadyChanged for me clears pendingMidGameClaim and rebuilds', () => {
    const s = baseUiState({ players: [{ token: MY, ready: false }] });
    const msg = { type: 'ReadyChanged', data: { token: MY, ready: true } };
    const r = routeMessage(msg, ctx(s, { pendingMidGameClaim: true }));
    expect(r.pendingMidGameClaim).toBe(false);
    expect(s.players[0].ready).toBe(true);
    expect(r.rebuildStations).toBe(true);
  });

  it('SpectatorChanged sets the role flag and rebuilds (issue #1105)', () => {
    const s = baseUiState({ players: [{ token: MY, spectator: false }] });
    const r = routeMessage({ type: 'SpectatorChanged', data: { token: MY, spectator: true } }, ctx(s));
    expect(s.players[0].spectator).toBe(true);
    expect(r.rebuildStations).toBe(true);
    // And the inverse toggle clears it.
    routeMessage({ type: 'SpectatorChanged', data: { token: MY, spectator: false } }, ctx(s));
    expect(s.players[0].spectator).toBe(false);
  });

  it('AfkChanged sets the presence flag and rebuilds (issue #1104)', () => {
    const s = baseUiState({ players: [{ token: MY, afk: false }] });
    const r = routeMessage({ type: 'AfkChanged', data: { token: MY, afk: true } }, ctx(s));
    expect(s.players[0].afk).toBe(true);
    expect(r.rebuildStations).toBe(true);
    // And the inverse toggle clears it — the presence delta only tracks the flag.
    routeMessage({ type: 'AfkChanged', data: { token: MY, afk: false } }, ctx(s));
    expect(s.players[0].afk).toBe(false);
  });

  it('NameChanged for my token surfaces the new name', () => {
    const s = baseUiState({ players: [{ token: MY, name: 'Old' }] });
    const r = routeMessage({ type: 'NameChanged', data: { token: MY, name: 'New' } }, ctx(s));
    expect(r.myName).toBe('New');
    expect(s.players[0].name).toBe('New');
  });

  it("NameChanged for another token leaves myName undefined", () => {
    const s = baseUiState({ players: [{ token: 'other', name: 'Old' }] });
    const r = routeMessage({ type: 'NameChanged', data: { token: 'other', name: 'New' } }, ctx(s));
    expect(r.myName).toBeUndefined();
  });

  it('PlayerJoined upserts by token in the msg fallback path', () => {
    const s = baseUiState({ players: [{ token: 'a', name: 'Ada' }] });
    routeMessage({ type: 'PlayerJoined', data: { player: { token: 'a', name: 'Ada2' } } }, ctx(s));
    expect(s.players).toHaveLength(1);
    expect(s.players[0].name).toBe('Ada2');
    routeMessage({ type: 'PlayerJoined', data: { player: { token: 'b', name: 'Bob' } } }, ctx(s));
    expect(s.players).toHaveLength(2);
  });

  it('PlayerLeft removes by token in the msg fallback path', () => {
    const s = baseUiState({ players: [{ token: 'a' }, { token: 'b' }] });
    routeMessage({ type: 'PlayerLeft', data: { token: 'a' } }, ctx(s));
    expect(s.players.map(p => p.token)).toEqual(['b']);
  });
});
