// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { LobbyState } from '../../gui/lobby-state.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { mergeReducerResults, REDUCER_EFFECTS } from '../../gui/reducer-result.js';
import { routeReducerResult } from '../../gui/client-router.js';
import { buildStationRoster } from '../../gui/station-roster.js';
import { lobbyViewModel } from '../../gui/lobby-view.js';
import { renderClientLobby, STATION_ROW_ATTR } from '../../gui/client-lobby-render.js';
import { prePlayView } from '../../gui/pre-play-view.js';

const html = fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), '../../client.html'), 'utf8');
const MY = '3f1a6c2e-0a11-4b3c-9d55-000000000071';
const OTHER = '3f1a6c2e-0a11-4b3c-9d55-000000000072';
const stations = {
  stations: [
    { id: 'helm', name: 'Helm', short_code: 'HLM', ratings: ['Std'] },
    { id: 'tactical', name: 'Tactical', short_code: 'TAC', ratings: ['Std'] },
  ],
};
const players = [
  { token: MY, name: 'Ada', connected: true, ready: true, station: 'helm' },
  { token: OTHER, name: 'Grace', connected: true, ready: true, station: 'tactical' },
];
const welcome = (crew) => ({
  type: 'Welcome',
  data: {
    state: { phase: 'Lobby', players: crew, world: { scenario_title: 'Retained mission' } },
    ship_stations: stations,
    ship_config: { helm_radar_range: 730, phaser_banks: [{ id: 'omni', cooldown_secs: 3 }] },
    station_ratings: { helm: 'Backfill', tactical: 'Backfill' },
    gms: [],
  },
});

function crewPage() {
  const lobby = new LobbyState();
  const sim = new ClientSimState();
  const ui = { players: [], stations: [], shipStations: stations, phase: 'Lobby' };
  const sent = [];
  function receive(message) {
    const result = routeReducerResult(mergeReducerResults(lobby.apply(message), sim.apply(message)), {
      uiState: ui, lobbyState: lobby, myToken: MY, pendingMidGameClaim: false, bezelAlertOn: false,
    });
    if (result.sideEffects.some(fx => fx.effect === REDUCER_EFFECTS.REBUILD_STATIONS)) {
      Object.assign(ui, buildStationRoster(ui.players, ui.shipStations.stations));
    }
    return result;
  }
  function paint() {
    const prePlay = prePlayView({}, {
      pickingScenario: lobby.showScenarioPicker(), waitingForScenario: lobby.waitingForScenario,
    }, null);
    if (!lobby.waitingForScenario) {
      renderClientLobby(document, lobbyViewModel(ui, MY, 'helm'), id => id, {
        claim: row => sent.push({ type: 'SelectStation', data: { station: row.name } }),
        setReady: ready => sent.push({ type: 'SetReady', data: { ready } }),
      });
    }
    return prePlay.surface;
  }
  function returnMessages() {
    receive({ type: 'GameStarted' });
    receive({ type: 'GameOver', data: { reason: 'Mission ended', report: [] } });
    for (const token of [MY, OTHER]) {
      receive({ type: 'StationAssigned', data: { token, station: null, station_id: null } });
      receive({ type: 'ReadyChanged', data: { token, ready: false } });
    }
    receive({ type: 'ReturnedToLobby' });
  }
  return { lobby, sim, ui, sent, receive, paint, returnMessages };
}

beforeEach(() => {
  const parsed = new DOMParser().parseFromString(html, 'text/html');
  document.body.replaceChildren(document.importNode(parsed.getElementById('lobby-ui'), true));
});

describe('retained native lobby completion through the shared crew projection', () => {
  it('repaints cleared crew and admits claim/Ready controls without reconnect or GameStarted', () => {
    const page = crewPage();
    page.receive(welcome(players));
    page.paint();
    expect(document.querySelector('#crew-display').textContent).toBe('2/2');
    expect(document.querySelector('#ready-pill').textContent).toBe('client.all_crew_ready');
    page.returnMessages();
    expect(page.paint()).toBe('waiting-overlay');

    // Native returns to the selected world's lobby and sends its real Welcome
    // shape after ReturnedToLobby, on the existing reliable connection.
    page.receive(welcome(players.map(player => ({ ...player, station: null, ready: false }))));
    expect(page.paint()).toBeNull();
    expect(page.ui.phase).toBe('Lobby');
    expect(page.lobby.players.map(player => player.token)).toEqual([MY, OTHER]);
    expect(page.lobby.scenarioTitle).toBe('Retained mission');
    expect(page.sim.helmRadarRange).toBe(730);
    expect(document.querySelector('#crew-display').textContent).toBe('0/2');
    expect(document.querySelector('#ready-pill').textContent).toBe('client.awaiting_crew');

    const claim = document.querySelector(`[${STATION_ROW_ATTR}="helm"] .claim-btn`);
    expect(claim.disabled).toBe(false);
    claim.click();
    expect(page.sent).toEqual([{ type: 'SelectStation', data: { station: 'Helm' } }]);
    page.receive({ type: 'StationAssigned', data: { token: MY, station: 'helm', station_id: 'helm' } });
    page.paint();
    const ready = document.querySelector('#ready-btn');
    expect(ready.style.display).toBe('block');
    expect(ready.disabled).toBe(false);
    ready.click();
    expect(page.sent[1]).toEqual({ type: 'SetReady', data: { ready: true } });
    page.receive({ type: 'ReadyChanged', data: { token: MY, ready: true } });
    page.paint();
    expect(ready.textContent).toBe('client.ready_confirmed');
    expect(page.ui.phase).toBe('Lobby');
  });

  it('keeps a browser crew waiting until genuine scenario selection completes', () => {
    const page = crewPage();
    page.receive(welcome(players));
    page.returnMessages();
    expect(page.paint()).toBe('waiting-overlay');
    page.receive({ type: 'ScenarioCatalog', data: { scenarios: [], locked_scenario: null, locked_ship: null } });
    expect(page.paint()).toBe('scenario-picker-overlay');
    expect(page.lobby.waitingForScenario).toBe(true);
    page.receive({ type: 'ScenarioCatalog', data: { scenarios: [], locked_scenario: 'next-world', locked_ship: 'next-hull' } });
    expect(page.paint()).toBeNull();
    expect(page.lobby.waitingForScenario).toBe(false);
    expect(page.ui.phase).toBe('Lobby');
  });

  it('requires the returned Welcome to reconnect a moved pane before its positive claim becomes a live seat', () => {
    const page = crewPage();
    const cleared = [
      { ...players[0], connected: false, station: null, ready: false },
      { ...players[1], station: 'helm', ready: false },
    ];
    page.receive(welcome(cleared));
    page.receive({ type: 'StationAssigned', data: { token: MY, station: 'Tactical', station_id: 'tactical' } });
    page.paint();
    // The observed failure was a positive assignment on a Session whose
    // GameOver-time Identify never ran. The ordinary roster rightly keeps a
    // disconnected holder's seat claimable; an assignment is not a handshake.
    expect(page.lobby.playerStation(MY)).toBe('tactical');
    expect(page.ui.players.find(player => player.token === MY).connected).toBe(false);
    expect(document.querySelector(`[${STATION_ROW_ATTR}="tactical"] .claim-btn`)).not.toBeNull();
    expect(document.querySelector('#crew-display').textContent).toBe('1/2');

    // With the lifecycle Identify accepted, the retained-return projection
    // carries the existing identity as connected and its station cleared.
    page.receive(welcome(cleared.map(player => ({ ...player, connected: true }))));
    page.paint();
    document.querySelector(`[${STATION_ROW_ATTR}="tactical"] .claim-btn`).click();
    expect(page.sent).toEqual([{ type: 'SelectStation', data: { station: 'Tactical' } }]);
    page.receive({ type: 'StationAssigned', data: { token: MY, station: 'Tactical', station_id: 'tactical' } });
    page.paint();
    expect(document.querySelector(`[${STATION_ROW_ATTR}="tactical"] .mine-btn`)).not.toBeNull();
    expect(document.querySelector('#crew-display').textContent).toBe('2/2');
    const ready = document.querySelector('#ready-btn');
    expect(ready.style.display).toBe('block');
    expect(ready.disabled).toBe(false);
    ready.click();
    expect(page.sent[1]).toEqual({ type: 'SetReady', data: { ready: true } });
    expect(page.ui.phase).toBe('Lobby');
  });
});
