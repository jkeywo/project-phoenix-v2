import { describe, expect, it } from 'vitest';
import {
  isSpectator,
  playerStation,
  routeReducerResult,
  showingLobbyDuringGame,
} from '../../gui/client-router.js';
import { ClientCommsState } from '../../gui/comms-state.js';
import { LobbyState } from '../../gui/lobby-state.js';
import {
  CHANGE_DOMAINS,
  REDUCER_EFFECTS,
  emptyReducerResult,
  mergeReducerResults,
} from '../../gui/reducer-result.js';
import { ClientSimState } from '../../gui/sim-state.js';

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
    ...overrides,
  };
}

function mirrorFrom(uiState) {
  const lobby = new LobbyState();
  lobby.phase = uiState.phase;
  lobby.players = uiState.players;
  lobby.shipStations = uiState.shipStations;
  lobby.countdownSecs = uiState.countdownSecs;
  lobby.gameOverReason = uiState.gameOverReason;
  return lobby;
}

const effectNames = result => result.sideEffects.map(effect => effect.effect);

describe('showingLobbyDuringGame', () => {
  it('suppresses a stationless or not-ready participant, but not a ready holder', () => {
    expect(showingLobbyDuringGame(baseUiState(), MY, false)).toBe(false);
    expect(showingLobbyDuringGame(baseUiState({ phase: 'InProgress' }), MY, false)).toBe(true);
    expect(showingLobbyDuringGame(baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: false }],
    }), MY, false)).toBe(true);
    expect(showingLobbyDuringGame(baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: true }],
    }), MY, false)).toBe(false);
  });

  it('does not suppress an explicit Spectator live-summary render', () => {
    const state = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: null, ready: false, spectator: true }],
    });
    expect(isSpectator(state, MY)).toBe(true);
    expect(showingLobbyDuringGame(state, MY, false)).toBe(false);
  });
});

describe('Welcome reducer effects', () => {
  const welcome = {
    type: 'Welcome',
    data: {
      state: {
        phase: 'Lobby',
        players: [{ token: MY, name: 'Ada', ready: false, station: null }],
        world: {
          scenario_title: 'Falling Skyway',
          scenario_description: 'Hold the line',
        },
      },
      ship_stations: {
        stations: [{ id: 'helm', console: 'gui/battleship/helm.html' }],
      },
      ship_config: { ship_css: 'gui/battleship/theme.css' },
    },
  };

  it('initialises the shell from reduced state without a message fallback', () => {
    const ui = baseUiState({
      phase: 'GameOver',
      players: [{ token: 'stale' }],
      countdownSecs: 9,
      gameOverReason: 'stale',
    });
    const lobby = new LobbyState();
    const sim = new ClientSimState();
    const merged = mergeReducerResults(lobby.apply(welcome), sim.apply(welcome));
    const routed = routeReducerResult(merged, ctx(ui, {
      lobbyState: lobby,
      bezelAlertOn: true,
    }));

    expect(ui.phase).toBe('Lobby');
    expect(ui.players).toBe(lobby.players);
    expect(playerStation(ui, MY)).toBeNull();
    expect(ui.shipStations).toBe(lobby.shipStations);
    expect(ui.countdownSecs).toBe(0);
    expect(ui.gameOverReason).toBeNull();
    expect(effectNames(routed)).toEqual([
      REDUCER_EFFECTS.MOUNT_CONSOLES,
      REDUCER_EFFECTS.SHIP_THEME,
      REDUCER_EFFECTS.SHIP_INFO,
      REDUCER_EFFECTS.STATUS,
      REDUCER_EFFECTS.REBUILD_STATIONS,
      REDUCER_EFFECTS.BEZEL_ALERT,
      REDUCER_EFFECTS.REPORT_ELIGIBILITY,
    ]);
    expect(routed.sideEffects[0].shipStations).toBe(lobby.shipStations);
    expect(routed.sideEffects[2]).toMatchObject({
      title: 'Falling Skyway', description: 'Hold the line',
    });
    expect(routed.sideEffects[5].on).toBe(false);
  });

  it('re-emits complete reconnect initialisation for an identical Welcome', () => {
    const lobby = new LobbyState();
    const first = lobby.apply(welcome);
    const second = lobby.apply(welcome);

    expect(first.effects).toEqual(second.effects);
    expect(first.effects.map(effect => effect.effect)).toEqual([
      REDUCER_EFFECTS.MOUNT_CONSOLES,
      REDUCER_EFFECTS.SHIP_THEME,
      REDUCER_EFFECTS.SHIP_INFO,
      REDUCER_EFFECTS.STATUS,
      REDUCER_EFFECTS.REBUILD_STATIONS,
    ]);
  });

  it('retains an active bezel through InProgress Welcome until Captain resync', () => {
    const reconnect = {
      ...welcome,
      data: {
        ...welcome.data,
        state: {
          ...welcome.data.state,
          phase: 'InProgress',
          players: [{ token: MY, name: 'Ada', ready: true, station: 'captain' }],
        },
      },
    };
    const ui = baseUiState();
    const lobby = new LobbyState();
    const sim = new ClientSimState();
    const welcomeResult = routeReducerResult(
      mergeReducerResults(lobby.apply(reconnect), sim.apply(reconnect)),
      ctx(ui, { lobbyState: lobby, bezelAlertOn: true }),
    );

    expect(welcomeResult.bezelAlertOn).toBe(true);
    expect(welcomeResult.sideEffects).not.toContainEqual({
      effect: REDUCER_EFFECTS.BEZEL_ALERT,
      on: false,
    });

    const captainResult = routeReducerResult(sim.apply({
      type: 'BlackboardUpdate',
      data: {
        updates: [['bridge-orders', { kind: 'Captain', data: { red_alert: false } }]],
      },
    }), ctx(ui, {
      lobbyState: lobby,
      bezelAlertOn: welcomeResult.bezelAlertOn,
    }));
    expect(captainResult.sideEffects).toContainEqual({
      effect: REDUCER_EFFECTS.BEZEL_ALERT,
      on: false,
    });
  });

  it('has no raw-payload fallback when the lobby reducer is unavailable', () => {
    const ui = baseUiState({ players: [{ token: 'stale' }] });
    const changed = emptyReducerResult();
    changed.changedDomains.add(CHANGE_DOMAINS.LOBBY);

    const routed = routeReducerResult(changed, ctx(ui));

    expect(ui.players).toEqual([{ token: 'stale' }]);
    expect(routed.sideEffects).toEqual([]);
  });
});

describe('ordered repeatable feedback', () => {
  it('keeps equal vibration effects from separate reductions', () => {
    const sim = new ClientSimState();
    const merged = mergeReducerResults(
      sim.apply({ type: 'DamageTaken', data: { hull: 30 } }),
      sim.apply({ type: 'DamageTaken', data: { hull: 30 } }),
    );
    const routed = routeReducerResult(merged, ctx(baseUiState()));

    expect(routed.sideEffects).toEqual([
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 300 },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 300 },
    ]);
  });

  it('keeps #1255 presentation envelopes and repeated Coordination displays', () => {
    const sim = new ClientSimState();
    const message = {
      type: 'CoordinationPopup',
      data: {
        address: { type: 'Station', data: 'tactical' },
        payload: { type: 'Alert' },
        presentation: { title: 'coordination.alert.title', body: 'Brace' },
        sender_label: 'chatter.sender.helm',
        to_label: 'station.tactical.name',
      },
    };
    const merged = mergeReducerResults(sim.apply(message), sim.apply(message));
    const routed = routeReducerResult(merged, ctx(baseUiState()));

    expect(routed.sideEffects).toHaveLength(2);
    for (const effect of routed.sideEffects) {
      expect(effect).toEqual({
        effect: REDUCER_EFFECTS.COORDINATION_POPUP,
        address: message.data.address,
        presentation: message.data.presentation,
        senderLabel: message.data.sender_label,
        targetLabel: message.data.to_label,
      });
      expect(effect).not.toHaveProperty('payload');
    }
  });

  it('ignores an unknown effect without swallowing later known feedback', () => {
    const result = emptyReducerResult();
    result.effects.push(
      { effect: 'future-shell-effect', payload: 1 },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 80 },
    );

    expect(routeReducerResult(result, ctx(baseUiState())).sideEffects)
      .toEqual([{ effect: REDUCER_EFFECTS.VIBRATE, duration: 80 }]);
  });
});

describe('render and bezel policy', () => {
  const midGameLobby = () => baseUiState({
    phase: 'InProgress',
    players: [{ token: MY, station: 'helm', ready: false }],
  });

  it('suppresses the mid-game lobby repaint but still applies bezel feedback', () => {
    const sim = new ClientSimState();
    const changes = sim.apply({
      type: 'BlackboardUpdate',
      data: {
        updates: [[
          'arbitrarily-named-command-system',
          { kind: 'Captain', data: { red_alert: true } },
        ]],
      },
    });
    const routed = routeReducerResult(changes, ctx(midGameLobby()));

    expect(routed.sideEffects).toEqual([
      { effect: REDUCER_EFFECTS.BEZEL_ALERT, on: true },
    ]);
    expect(routed.bezelAlertOn).toBe(true);
  });

  it('uses the typed Captain blackboard kind, not a literal System id', () => {
    const sim = new ClientSimState();
    const captain = sim.apply({
      type: 'BlackboardUpdate',
      data: { updates: [['bridge-orders', { kind: 'Captain', data: { red_alert: true } }]] },
    });
    const unrelated = sim.apply({
      type: 'BlackboardUpdate',
      data: { updates: [['captain', { kind: 'Helm', data: { yaw: 0 } }]] },
    });

    expect(captain.effects).toContainEqual({
      effect: REDUCER_EFFECTS.BEZEL_ALERT, on: true,
    });
    expect(unrelated.effects.some(effect => effect.effect === REDUCER_EFFECTS.BEZEL_ALERT))
      .toBe(false);
  });

  it('filters repeated bezel observations but not other feedback', () => {
    const result = emptyReducerResult();
    result.effects.push(
      { effect: REDUCER_EFFECTS.BEZEL_ALERT, on: true },
      { effect: REDUCER_EFFECTS.BEZEL_ALERT, on: true },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 60 },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 60 },
    );

    const routed = routeReducerResult(result, ctx(baseUiState()));
    expect(routed.sideEffects).toEqual([
      { effect: REDUCER_EFFECTS.BEZEL_ALERT, on: true },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 60 },
      { effect: REDUCER_EFFECTS.VIBRATE, duration: 60 },
    ]);
  });

  it('renders a seated ready player from reducer-owned simulation and Comms effects', () => {
    const ui = baseUiState({
      phase: 'InProgress',
      players: [{ token: MY, station: 'helm', ready: true }],
    });
    const sim = new ClientSimState();
    const comms = new ClientCommsState();

    expect(effectNames(routeReducerResult(
      sim.apply({ type: 'SimState', data: { snapshot: {} } }), ctx(ui),
    ))).toEqual([REDUCER_EFFECTS.REQUEST_RENDER]);
    expect(effectNames(routeReducerResult(
      comms.apply({ type: 'CommsState', data: {} }), ctx(ui),
    ))).toEqual([REDUCER_EFFECTS.REQUEST_RENDER]);
  });
});

describe('lobby lifecycle effects', () => {
  it('settles and force-renders a ScenarioCatalog without a shell message exception', () => {
    const lobby = new LobbyState();
    const changes = lobby.apply({
      type: 'ScenarioCatalog',
      data: { scenarios: [], locked_scenario: null, locked_ship: null },
    });
    const routed = routeReducerResult(changes, ctx(baseUiState(), { lobbyState: lobby }));

    expect(routed.sideEffects).toEqual([
      { effect: REDUCER_EFFECTS.SETTLE_SCENARIO_PICK },
      { effect: REDUCER_EFFECTS.REQUEST_RENDER, force: true },
    ]);
  });

  it('routes loading progress and GameStarted overlay lifecycle', () => {
    const lobby = new LobbyState();
    const progress = routeReducerResult(
      lobby.apply({ type: 'LoadingProgress', data: { fraction: 0.427 } }),
      ctx(baseUiState(), { lobbyState: lobby }),
    );
    expect(progress.sideEffects).toEqual([
      { effect: REDUCER_EFFECTS.SHOW_LOADING, pct: 43 },
      { effect: REDUCER_EFFECTS.REQUEST_RENDER },
    ]);

    const started = routeReducerResult(
      lobby.apply({ type: 'GameStarted' }),
      ctx(baseUiState(), { lobbyState: lobby }),
    );
    // Once GameStarted folds the empty fixture into InProgress, the stationless
    // client is showing the mid-game lobby: hide the loader, but suppress the
    // full repaint that would tear down its claim controls.
    expect(started.sideEffects).toEqual([{ effect: REDUCER_EFFECTS.HIDE_LOADING }]);
  });

  it('updates local claim/name state from reducer effects', () => {
    const lobby = new LobbyState();
    lobby.players = [{ token: MY, name: 'Old', station: 'helm', ready: false }];
    const ui = baseUiState({ players: lobby.players });

    const renamed = routeReducerResult(
      lobby.apply({ type: 'NameChanged', data: { token: MY, name: 'New' } }),
      ctx(ui, { lobbyState: lobby }),
    );
    expect(renamed.myName).toBe('New');
    expect(effectNames(renamed)).toEqual([REDUCER_EFFECTS.REBUILD_STATIONS]);

    const ready = routeReducerResult(
      lobby.apply({ type: 'ReadyChanged', data: { token: MY, ready: true } }),
      ctx(ui, { lobbyState: lobby, pendingMidGameClaim: true }),
    );
    expect(ready.pendingMidGameClaim).toBe(false);

    const released = routeReducerResult(
      lobby.apply({ type: 'StationAssigned', data: { token: MY, station_id: null } }),
      ctx(ui, { lobbyState: lobby, pendingMidGameClaim: true }),
    );
    expect(released.pendingMidGameClaim).toBe(false);
  });

  it('only Welcome asks to mount consoles', () => {
    const lobby = new LobbyState();
    lobby.players = [{ token: MY, station: null }];
    for (const message of [
      { type: 'PlayerJoined', data: { player: { token: 'other' } } },
      { type: 'StationAssigned', data: { token: MY, station_id: 'helm' } },
      { type: 'ReadyChanged', data: { token: MY, ready: true } },
    ]) {
      const changes = lobby.apply(message);
      expect(changes.effects.some(effect => effect.effect === REDUCER_EFFECTS.MOUNT_CONSOLES))
        .toBe(false);
    }
  });
});
