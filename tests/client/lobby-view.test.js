import { describe, it, expect } from 'vitest';
import { lobbyViewModel, nextLobbyConsole } from '../../gui/lobby-view.js';

const MY = 'tok-me';

function uiState(overrides = {}) {
  return {
    players: [],
    stations: [],
    maxPlayers: 0,
    allReady: false,
    countdownSecs: 0,
    phase: 'Lobby',
    ...overrides,
  };
}

const helmRow = (overrides = {}) => ({
  id: 'helm', name: 'Helm', short_code: 'HLM', rank: 'Lt',
  holder_name: null, holder_token: null, ratings: ['Std'],
  ...overrides,
});

describe('nextLobbyConsole — detail-panel auto-select', () => {
  it('auto-selects the single console of my station', () => {
    expect(nextLobbyConsole(null, { id: 'helm' })).toBe('helm');
  });

  it('keeps an existing matching selection', () => {
    expect(nextLobbyConsole('helm', { id: 'helm' })).toBe('helm');
  });

  it('re-targets when the held station changes', () => {
    expect(nextLobbyConsole('helm', { id: 'captain' })).toBe('captain');
  });

  it('clears the selection when I hold no station', () => {
    expect(nextLobbyConsole('helm', null)).toBeNull();
  });
});

describe('lobbyViewModel — rows', () => {
  it('computes row class, glyph, button kind and occupant', () => {
    const s = uiState({
      players: [{ token: MY, name: 'Ada', ready: false }],
      stations: [
        helmRow({ holder_name: 'Ada', holder_token: MY }),
        helmRow({ id: 'captain', name: 'Captain', short_code: 'c', holder_name: 'Bob', holder_token: 'other' }),
        helmRow({ id: 'comms', name: 'Comms', short_code: '' }),
      ],
    });
    const vm = lobbyViewModel(s, MY, null);
    expect(vm.rows.map(r => r.button)).toEqual(['release', 'taken', 'claim']);
    expect(vm.rows[0].rowClass).toBe('station-row mine');
    expect(vm.rows[1].rowClass).toBe('station-row taken');
    expect(vm.rows[2].rowClass).toBe('station-row');
    expect(vm.rows[0].glyph).toBe('HL');
    expect(vm.rows[1].glyph).toBe('C');
    expect(vm.rows[2].glyph).toBe('--');
    expect(vm.rows[1].occupant).toBe('Bob');
    expect(vm.rows[0].occupant).toBeNull();
  });

  it('resolves labels through the injected labelFor', () => {
    const s = uiState({ stations: [helmRow()] });
    const vm = lobbyViewModel(s, MY, null, { labelFor: st => 'L:' + st.id });
    expect(vm.rows[0].label).toBe('L:helm');
  });
});

describe('lobbyViewModel — has-station / detail panel', () => {
  const seated = () => uiState({
    players: [{ token: MY, name: 'Ada', ready: false }],
    stations: [helmRow({ holder_name: 'Ada', holder_token: MY, ratings: ['Std', 'Simplified'] })],
  });

  it('hasStation + active detail with auto-selected console', () => {
    const vm = lobbyViewModel(seated(), MY, null);
    expect(vm.hasStation).toBe(true);
    expect(vm.detail.active).toBe(true);
    expect(vm.detail.consoles).toEqual(['helm']);
    expect(vm.selectedConsole).toBe('helm');
  });

  it('spectator: idle detail, null selection', () => {
    const vm = lobbyViewModel(uiState({ stations: [helmRow()] }), MY, 'helm');
    expect(vm.hasStation).toBe(false);
    expect(vm.detail.active).toBe(false);
    expect(vm.selectedConsole).toBeNull();
  });

  it('multi-rating stations expose the ratings block with the active rating', () => {
    const vm = lobbyViewModel(seated(), MY, null, {
      stationRatings: { helm: 'Simplified' },
    });
    expect(vm.detail.ratings).toEqual({ list: ['Std', 'Simplified'], active: 'Simplified' });
  });

  it('single-rating stations omit the ratings block; first rating is the default active', () => {
    const s = seated();
    s.stations[0].ratings = ['Std'];
    expect(lobbyViewModel(s, MY, null).detail.ratings).toBeNull();
    const vm = lobbyViewModel(seated(), MY, null, { stationRatings: {} });
    expect(vm.detail.ratings.active).toBe('Std');
  });
});

describe('lobbyViewModel — ready button', () => {
  const seated = (playerOverrides = {}, stateOverrides = {}) => uiState({
    players: [{ token: MY, name: 'Ada', ready: false, ...playerOverrides }],
    stations: [helmRow({ holder_name: 'Ada', holder_token: MY })],
    ...stateOverrides,
  });

  it('hidden for spectators', () => {
    const vm = lobbyViewModel(uiState({ stations: [helmRow()] }), MY, null);
    expect(vm.readyBtn.visible).toBe(false);
  });

  it('ready mode when seated and not ready — pressing sends ready:true', () => {
    const vm = lobbyViewModel(seated(), MY, null);
    expect(vm.readyBtn).toEqual({ visible: true, mode: 'ready', sendReady: true });
  });

  it('ready-confirmed mode when ready — pressing un-readies', () => {
    const vm = lobbyViewModel(seated({ ready: true }), MY, null);
    expect(vm.readyBtn).toEqual({ visible: true, mode: 'ready-confirmed', sendReady: false });
  });

  it('countdown mode wins over ready state and carries the seconds', () => {
    const vm = lobbyViewModel(seated({ ready: true }, { countdownSecs: 5 }), MY, null);
    expect(vm.readyBtn).toEqual({ visible: true, mode: 'countdown', secs: 5, sendReady: false });
  });
});

describe('lobbyViewModel — status line string-id selection', () => {
  const seated = (playerOverrides = {}, stateOverrides = {}) => uiState({
    players: [{ token: MY, name: 'Ada', ready: false, ...playerOverrides }],
    stations: [helmRow({ holder_name: 'Ada', holder_token: MY })],
    ...stateOverrides,
  });
  const labelFor = st => 'Helm';

  it('select-station when I have no station', () => {
    const vm = lobbyViewModel(uiState(), MY, null);
    expect(vm.statusLine).toEqual({ id: 'client.status_select_station', params: {} });
  });

  it('launching with secs during countdown', () => {
    const vm = lobbyViewModel(seated({}, { countdownSecs: 4 }), MY, null, { labelFor });
    expect(vm.statusLine).toEqual({ id: 'client.status_launching', params: { secs: 4 } });
  });

  it('all-ready beats waiting-crew', () => {
    const vm = lobbyViewModel(seated({ ready: true }, { allReady: true }), MY, null);
    expect(vm.statusLine.id).toBe('client.status_all_ready');
  });

  it('waiting-crew when I am ready but others are not', () => {
    const vm = lobbyViewModel(seated({ ready: true }), MY, null);
    expect(vm.statusLine.id).toBe('client.status_waiting_crew');
  });

  it('standing-by with the station label otherwise', () => {
    const vm = lobbyViewModel(seated(), MY, null, { labelFor });
    expect(vm.statusLine).toEqual({ id: 'client.status_standing_by', params: { station: 'Helm' } });
  });
});

describe('lobbyViewModel — crew counter', () => {
  it('counts held stations over maxPlayers', () => {
    const s = uiState({
      maxPlayers: 3,
      stations: [helmRow({ holder_name: 'Ada' }), helmRow({ id: 'captain' })],
    });
    expect(lobbyViewModel(s, MY, null).crew).toEqual({ filled: 1, max: 3 });
  });
});
