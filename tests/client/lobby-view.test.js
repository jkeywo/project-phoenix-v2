import { describe, it, expect } from 'vitest';
import { lobbyViewModel, nextLobbyConsole, releaseConfirmStep } from '../../gui/lobby-view.js';

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

// PRD #1023 module 4 / user story 2: "I want to read what a station does
// before I claim it". The contract is therefore about a row whose button is
// 'claim' — an unclaimed seat that already carries its copy.
describe('lobbyViewModel — station descriptions before the claim', () => {
  it('carries a description on a free (claimable) row', () => {
    const s = uiState({
      stations: [helmRow({ description: 'Pilot the ship.' })],
    });
    const vm = lobbyViewModel(s, MY, null);
    expect(vm.rows[0].button).toBe('claim');
    expect(vm.rows[0].description).toBe('Pilot the ship.');
  });

  it('carries a description on taken and mine rows too', () => {
    const s = uiState({
      players: [{ token: MY, name: 'Ada', ready: false }],
      stations: [
        helmRow({ holder_name: 'Ada', holder_token: MY, description: 'Pilot the ship.' }),
        helmRow({ id: 'captain', holder_name: 'Bob', holder_token: 'other', description: 'Command the ship.' }),
      ],
    });
    const vm = lobbyViewModel(s, MY, null);
    expect(vm.rows.map(r => r.button)).toEqual(['release', 'taken']);
    expect(vm.rows.map(r => r.description)).toEqual(['Pilot the ship.', 'Command the ship.']);
  });

  it('resolves descriptions through the injected describeFor', () => {
    // client.html injects a resolver that turns the wire's string id into
    // authored copy; the view model must not assume the raw field is prose.
    const s = uiState({ stations: [helmRow({ description: 'station.helm.description' })] });
    const vm = lobbyViewModel(s, MY, null, { describeFor: st => 'D:' + st.description });
    expect(vm.rows[0].description).toBe('D:station.helm.description');
  });

  it('is an empty string when the hull authored none', () => {
    const vm = lobbyViewModel(uiState({ stations: [helmRow()] }), MY, null);
    expect(vm.rows[0].description).toBe('');
  });

  it('restates the held station description on the detail panel', () => {
    const s = uiState({
      players: [{ token: MY, name: 'Ada', ready: false }],
      stations: [helmRow({ holder_name: 'Ada', holder_token: MY, description: 'Pilot the ship.' })],
    });
    expect(lobbyViewModel(s, MY, null).detail.stationDescription).toBe('Pilot the ship.');
    // Idle detail carries the field so the caller can clear the node
    // unconditionally rather than branching on presence.
    const idle = lobbyViewModel(uiState({ stations: [helmRow()] }), MY, null);
    expect(idle.detail.stationDescription).toBe('');
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

  it('take-station mode in-progress — same SetReady{true} hand-off (#771 AC1/AC2)', () => {
    const vm = lobbyViewModel(seated({}, { phase: 'InProgress' }), MY, null);
    expect(vm.readyBtn).toEqual({ visible: true, mode: 'take-station', sendReady: true });
  });

  it('lobby keeps plain ready mode (not take-station)', () => {
    const vm = lobbyViewModel(seated({}, { phase: 'Lobby' }), MY, null);
    expect(vm.readyBtn.mode).toBe('ready');
  });
});

describe('releaseConfirmStep — mid-round release arm→confirm (#771 AC3/AC4)', () => {
  it('lobby release is immediate — sends without arming', () => {
    expect(releaseConfirmStep('Lobby', false)).toEqual({ send: true, armed: false });
  });

  it('in-progress first click arms and does not send', () => {
    expect(releaseConfirmStep('InProgress', false)).toEqual({ send: false, armed: true });
  });

  it('in-progress second click (armed) sends and disarms', () => {
    expect(releaseConfirmStep('InProgress', true)).toEqual({ send: true, armed: false });
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
