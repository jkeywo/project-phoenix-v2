import { describe, it, expect } from 'vitest';
import { hostLobbyViewModel } from '../../gui/host-lobby-view.js';

function payload(overrides = {}) {
  return {
    phase: 'Lobby',
    scenario_title: 'Combat Test',
    scenario_body: 'A shakedown run.',
    crew_count: 0,
    max_players: 0,
    all_ready: false,
    stations: [],
    spectators: [],
    countdown_secs: 0,
    ...overrides,
  };
}

const helmStation = (overrides = {}) => ({
  name: 'Helm', short_code: 'HLM', rank: 'Lt',
  holder_name: null, preset_names: [],
  ...overrides,
});

describe('hostLobbyViewModel — phase transitions', () => {
  it('shows the loading overlay with a rounded percentage while Loading', () => {
    const vm = hostLobbyViewModel(payload({ phase: 'Loading', loading_progress: 0.4567 }), 'Lobby');
    expect(vm.transitions.showLoadingOverlay).toBe(true);
    expect(vm.transitions.loadingPct).toBe('46%');
  });

  it('omits the percentage when loading_progress is not a number', () => {
    const vm = hostLobbyViewModel(payload({ phase: 'Loading' }), 'Lobby');
    expect(vm.transitions.loadingPct).toBeNull();
  });

  it('dismisses the loading overlay only on the Loading -> InProgress edge', () => {
    const vm = hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Loading');
    expect(vm.transitions.dismissLoadingOverlay).toBe(true);
  });

  it('does not dismiss the loading overlay entering InProgress from Lobby', () => {
    const vm = hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Lobby');
    expect(vm.transitions.dismissLoadingOverlay).toBe(false);
  });

  it('unlocks audio on any fresh entry into InProgress, not just from Loading', () => {
    const fromLobby = hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Lobby');
    const fromLoading = hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Loading');
    expect(fromLobby.transitions.unlockAudio).toBe(true);
    expect(fromLoading.transitions.unlockAudio).toBe(true);
  });

  it('does not re-unlock audio while already InProgress', () => {
    const vm = hostLobbyViewModel(payload({ phase: 'InProgress' }), 'InProgress');
    expect(vm.transitions.unlockAudio).toBe(false);
  });

  it('starts menu music in Lobby, stops it in Loading/InProgress, leaves GameOver alone', () => {
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby').transitions.menuMusic).toBe('start');
    expect(hostLobbyViewModel(payload({ phase: 'Loading' }), 'Lobby').transitions.menuMusic).toBe('stop');
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Loading').transitions.menuMusic).toBe('stop');
    expect(hostLobbyViewModel(payload({ phase: 'GameOver' }), 'InProgress').transitions.menuMusic).toBeNull();
  });

  it('shows the panel only in Lobby', () => {
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby').transitions.showPanel).toBe(true);
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Lobby').transitions.showPanel).toBe(false);
  });

  it('QR overlay: show in Lobby, hide in Loading/GameOver, untouched in InProgress', () => {
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby').transitions.qrOverlayAction).toBe('show');
    expect(hostLobbyViewModel(payload({ phase: 'Loading' }), 'Lobby').transitions.qrOverlayAction).toBe('hide');
    expect(hostLobbyViewModel(payload({ phase: 'GameOver' }), 'InProgress').transitions.qrOverlayAction).toBe('hide');
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Loading').transitions.qrOverlayAction).toBeNull();
  });

  it('hides the game-over overlay whenever the phase is not GameOver', () => {
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'GameOver').transitions.hideGameOverOverlay).toBe(true);
    expect(hostLobbyViewModel(payload({ phase: 'GameOver' }), 'InProgress').transitions.hideGameOverOverlay).toBe(false);
  });
});

describe('hostLobbyViewModel — title / subtitle', () => {
  it('passes through the authored title and body', () => {
    const vm = hostLobbyViewModel(payload({ scenario_title: 'Falling Skyway', scenario_body: 'Storm inbound.' }), '');
    expect(vm.title).toBe('Falling Skyway');
    expect(vm.subtitle).toBe('Storm inbound.');
  });

  it('title falls back to null (glue applies the unknown-scenario string) when blank', () => {
    const vm = hostLobbyViewModel(payload({ scenario_title: '' }), '');
    expect(vm.title).toBeNull();
  });

  it('subtitle falls back to empty string when blank', () => {
    const vm = hostLobbyViewModel(payload({ scenario_body: '' }), '');
    expect(vm.subtitle).toBe('');
  });
});

describe('hostLobbyViewModel — crew counter', () => {
  it('counts crew against max and builds one dot per slot', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 2, max_players: 4 }), '');
    expect(vm.crew.count).toBe(2);
    expect(vm.crew.max).toBe(4);
    expect(vm.crew.dots).toEqual([true, true, false, false]);
  });

  it('shows the spectator tag only when spectators are present', () => {
    const none = hostLobbyViewModel(payload({ spectators: [] }), '');
    expect(none.crew.spectatorTag).toEqual({ visible: false, count: 0 });
    const some = hostLobbyViewModel(payload({ spectators: ['Riko', 'Zed'] }), '');
    expect(some.crew.spectatorTag).toEqual({ visible: true, count: 2 });
  });
});

describe('hostLobbyViewModel — ready badge', () => {
  it('countdown wins and carries the seconds', () => {
    const vm = hostLobbyViewModel(payload({ countdown_secs: 5, all_ready: true }), '');
    expect(vm.readyBadge).toEqual({ id: 'server.launching_in', params: { secs: 5 }, className: 'go' });
  });

  it('all-ready when no countdown', () => {
    const vm = hostLobbyViewModel(payload({ all_ready: true }), '');
    expect(vm.readyBadge).toEqual({ id: 'client.all_crew_ready', params: {}, className: 'go' });
  });

  it('awaiting crew otherwise', () => {
    const vm = hostLobbyViewModel(payload(), '');
    expect(vm.readyBadge).toEqual({ id: 'client.awaiting_crew', params: {}, className: '' });
  });
});

describe('hostLobbyViewModel — countdown display', () => {
  it('hidden at zero', () => {
    expect(hostLobbyViewModel(payload({ countdown_secs: 0 }), '').countdown).toEqual({ visible: false, secs: 0 });
  });

  it('visible with the remaining seconds', () => {
    expect(hostLobbyViewModel(payload({ countdown_secs: 7 }), '').countdown).toEqual({ visible: true, secs: 7 });
  });
});

describe('hostLobbyViewModel — station cards', () => {
  it('builds one card per station, claimed vs open, with initials', () => {
    const vm = hostLobbyViewModel(payload({
      stations: [
        helmStation({ holder_name: 'Ada' }),
        helmStation({ name: 'Captain', short_code: 'CAP', holder_name: null }),
      ],
    }), '');
    expect(vm.cards).toHaveLength(2);
    expect(vm.cards[0].claimed).toBe(true);
    expect(vm.cards[0].avatar).toEqual({ text: 'AD', placeholder: false });
    expect(vm.cards[1].claimed).toBe(false);
    expect(vm.cards[1].avatar).toEqual({ text: 'CA', placeholder: true });
  });

  it('falls back to -- when an open station has no short_code', () => {
    const vm = hostLobbyViewModel(payload({ stations: [helmStation({ short_code: '' })] }), '');
    expect(vm.cards[0].avatar).toEqual({ text: '--', placeholder: true });
  });

  it('uses short_code as the display name when name is blank', () => {
    const vm = hostLobbyViewModel(payload({ stations: [helmStation({ name: '', short_code: 'HLM' })] }), '');
    expect(vm.cards[0].name).toBe('HLM');
  });

  it('maps preset_names to pill string-ids, tagging Low', () => {
    const vm = hostLobbyViewModel(payload({
      stations: [helmStation({ preset_names: ['Low', 'Standard'] })],
    }), '');
    expect(vm.cards[0].presetPills).toEqual([
      { low: true, id: 'server.complexity_low' },
      { low: false, id: 'server.complexity_normal' },
    ]);
  });

  it('has no preset pills when the station authored none', () => {
    const vm = hostLobbyViewModel(payload({ stations: [helmStation()] }), '');
    expect(vm.cards[0].presetPills).toEqual([]);
  });
});

describe('hostLobbyViewModel — reserved chip', () => {
  it('is inactive: the grid is always sized to the roster', () => {
    const vm = hostLobbyViewModel(payload({ stations: [helmStation(), helmStation({ name: 'Comms' })] }), '');
    expect(vm.reservedChip).toEqual({ active: false, id: null, params: {} });
  });
});

describe('hostLobbyViewModel — spectator pill list', () => {
  it('one pill per claimed station, holder name and station name', () => {
    const vm = hostLobbyViewModel(payload({
      stations: [helmStation({ holder_name: 'Ada' }), helmStation({ name: 'Captain', holder_name: 'Bo' })],
    }), '');
    expect(vm.spectatorPills).toEqual([
      { kind: 'crew', text: 'Ada · Helm' },
      { kind: 'crew', text: 'Bo · Captain' },
    ]);
  });

  it('appends a waiting pill per explicit spectator', () => {
    const vm = hostLobbyViewModel(payload({
      stations: [helmStation({ holder_name: 'Ada' })],
      spectators: ['Zed'],
    }), '');
    expect(vm.spectatorPills).toEqual([
      { kind: 'crew', text: 'Ada · Helm' },
      { kind: 'waiting', id: 'server.spectator_waiting', params: { name: 'Zed' } },
    ]);
  });

  it('is a single empty-state entry when nobody has joined', () => {
    const vm = hostLobbyViewModel(payload(), '');
    expect(vm.spectatorPills).toEqual([{ kind: 'empty', id: 'server.no_players', params: {} }]);
  });
});

describe('hostLobbyViewModel — status hint', () => {
  it('launching hint wins during countdown', () => {
    const vm = hostLobbyViewModel(payload({ countdown_secs: 3 }), '');
    expect(vm.hint).toEqual({ id: 'server.hint_launching', params: { secs: 3 }, color: '#5fd8e8' });
  });

  it('waiting-for-players when nobody has joined', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 0 }), '');
    expect(vm.hint).toEqual({ id: 'server.waiting_players', params: {}, color: '#667' });
  });

  it('all-ready hint when the crew is ready', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 1, all_ready: true }), '');
    expect(vm.hint).toEqual({ id: 'client.status_all_ready', params: {}, color: '#5fd8e8' });
  });

  it('waiting-for-ready otherwise', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 1, all_ready: false }), '');
    expect(vm.hint).toEqual({ id: 'server.waiting_ready', params: {}, color: '#889' });
  });
});

describe('hostLobbyViewModel — AI-only launch button', () => {
  it('visible only when no crew and no spectators', () => {
    expect(hostLobbyViewModel(payload(), '').aiLaunchVisible).toBe(true);
    expect(hostLobbyViewModel(payload({ crew_count: 1 }), '').aiLaunchVisible).toBe(false);
    expect(hostLobbyViewModel(payload({ spectators: ['Zed'] }), '').aiLaunchVisible).toBe(false);
  });
});
