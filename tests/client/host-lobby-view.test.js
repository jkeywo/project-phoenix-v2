import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  hostLobbyViewModel,
  hostLobbyMonitorRow,
  hostLobbyStationRows,
  fleetStartValidationState,
  joinPanelAction,
  joinPanelSuppressed,
} from '../../gui/host-lobby-view.js';
// The real assets/strings/strings.csv is loaded by the vitest setup file, so the
// tests below can check the sentence an operator actually reads and not only the
// id and the params it is built from.
import { t } from '../../gui/strings.js';

const SERVER_HTML = fs.readFileSync(path.join(
  path.dirname(fileURLToPath(import.meta.url)), '../../server.html',
), 'utf-8');

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
    gms: [],
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
    // The law BOTH surfaces obey since issue #1329: the native host's lobby
    // document applies this same action through the same gui/host-qr.js the
    // host page applies it through, so there is one answer here rather than a
    // native opinion about when a QR is on screen.
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby').transitions.qrOverlayAction).toBe('show');
    expect(hostLobbyViewModel(payload({ phase: 'Loading' }), 'Lobby').transitions.qrOverlayAction).toBe('hide');
    expect(hostLobbyViewModel(payload({ phase: 'GameOver' }), 'InProgress').transitions.qrOverlayAction).toBe('hide');
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Loading').transitions.qrOverlayAction).toBeNull();
    // Every entry into InProgress, not only the one from Loading: a direct
    // start never passes through it, and a `hide` here would shut the panel an
    // operator had opened. This is the case the native surface leans on hardest
    // — its only in-play control is behind F9, so a spurious hide is a QR the
    // operator cannot get back without noticing it went.
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'Lobby').transitions.qrOverlayAction).toBeNull();
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'InProgress').transitions.qrOverlayAction).toBeNull();
  });

  it('keeps the join panel off the landing, on a host whose phase already reads Lobby', () => {
    // The bug this input exists for. `GamePhase::Lobby` is the DEFAULT
    // (src/core/messages.rs), so a world-less native host boots straight into
    // it and the phase law answered 'show' while the operator was still at the
    // landing's front door with no World chosen — the join code drawn over the
    // menu, above it in the stacking order. The landing is not a phase, so the
    // view model is told about it rather than asked to infer it.
    const landing = { landingUp: true };
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), '', null, landing)
      .transitions.qrOverlayAction).toBe('hide');
    // …and it is a 'hide', not a "leave it alone": the host page docks the same
    // node into #scenario-panel before any of this runs, so the panel can
    // already be visible by the time the law is first consulted.
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'InProgress', null, landing)
      .transitions.qrOverlayAction).toBe('hide');
  });

  it('shows it again the moment the landing is dismissed', () => {
    // Dismissal is what "the lobby is open" means on a surface that has a
    // landing; nothing else about the law changed.
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby', null, { landingUp: false })
      .transitions.qrOverlayAction).toBe('show');
    // A caller with no landing at all — and the pre-#1355 three-argument call —
    // gets the phase law on its own.
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby', null, {})
      .transitions.qrOverlayAction).toBe('show');
  });

  it('is the phase law for a host that never had a landing at all', () => {
    // Not the same surface as "the landing was dismissed", and the difference
    // is the whole of `phoenix-host --world …`: `feed_landing_panel` requires a
    // `LobbyScenarioCatalog`, which src/native_host/app.rs inserts only when
    // the process was started WITHOUT a world, so that host never publishes a
    // landing — while `feed_lobby_state` has no such gate and its lobby pushes
    // arrive as usual. A surface that reported “landing up” off a default
    // nothing had answered would take the QR, the URL and the typed code away
    // for the whole run, and re-hide the panel on every push behind the
    // operator's own toggle. That is why the native document pairs
    // `landingState.dismissed` with `landingPushed` and this reads 'show'.
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), 'Lobby', null, { landingUp: false })
      .transitions.qrOverlayAction).toBe('show');
    expect(hostLobbyViewModel(payload({ phase: 'InProgress' }), 'InProgress', null,
      { landingUp: false }).transitions.qrOverlayAction).toBeNull();
  });

  it('shows on a landing that is carrying the panel (issue #755 AC1)', () => {
    // The host page docks #overlay into the World picker the landing borrows
    // into its middle column, so the panel cannot cover the front door — and a
    // crew scans the code from the same column the room is picking a World in.
    // Neither landing row consults the phase, which matters here: no world is
    // loaded, so the page's `_lobbyPrevPhase` is still '' and a fall-through
    // would answer null for the whole of selection.
    const docked = { landingUp: true, panelDocked: true };
    expect(hostLobbyViewModel(payload({ phase: 'Lobby' }), '', null, docked)
      .transitions.qrOverlayAction).toBe('show');
    expect(hostLobbyViewModel(payload({ phase: '' }), '', null, docked)
      .transitions.qrOverlayAction).toBe('show');
    // Round two, after a Game Over returns the page to the picker.
    expect(hostLobbyViewModel(payload({ phase: 'GameOver' }), 'GameOver', null, docked)
      .transitions.qrOverlayAction).toBe('show');
  });

  it('states the whole law once, so both surfaces read the same table', () => {
    // The rows in gui/host-qr.js's header, top to bottom: a landing CARRYING
    // the panel shows it, a landing standing in front of it hides it whatever
    // the phase says, then Lobby shows, Loading/GameOver hide, and InProgress
    // is left alone so an operator's toggle survives a mission.
    expect(joinPanelAction('Lobby', true, true)).toBe('show');
    expect(joinPanelAction('GameOver', true, true)).toBe('show');
    expect(joinPanelAction('', true, true)).toBe('show');
    expect(joinPanelAction('Lobby', true)).toBe('hide');
    expect(joinPanelAction('Loading', true)).toBe('hide');
    expect(joinPanelAction('InProgress', true)).toBe('hide');
    expect(joinPanelAction('Lobby', false)).toBe('show');
    expect(joinPanelAction('Loading', false)).toBe('hide');
    expect(joinPanelAction('GameOver', false)).toBe('hide');
    expect(joinPanelAction('InProgress', false)).toBeNull();
    // The native document's first render, before any phase has arrived.
    expect(joinPanelAction('', false)).toBeNull();
    // A surface that never docks reads `panelDocked` false however it asks.
    expect(joinPanelAction('Lobby', true, false)).toBe('hide');
    expect(joinPanelAction('Lobby', true, undefined)).toBe('hide');
  });

  it('answers the toggles with the same fact it answers the law with', () => {
    // `joinPanelSuppressed` is the row every toggle obeys — the host page's cog
    // and a phone's ToggleQrCode (`joinPanelBlockedByLanding` in server.html),
    // the native surface's control and verb (`requestToggleQr`). It is one
    // function because "not on screen, and not the operator's to open either"
    // is one fact: nothing reasserts the law during a mission, both hosts dedupe
    // their lobby push, and the landing only re-asks when it MOVES, so a press
    // over the front door would stick until the roster happened to change.
    expect(joinPanelSuppressed(true)).toBe(true);
    expect(joinPanelSuppressed(true, false)).toBe(true);
    // …and it does NOT refuse the docked panel, which is on screen on purpose.
    expect(joinPanelSuppressed(true, true)).toBe(false);
    expect(joinPanelSuppressed(false)).toBe(false);
    expect(joinPanelSuppressed(false, true)).toBe(false);
    // Whenever it refuses, the law hides — whatever the phase says. That pair
    // is what makes gui/host-qr.js's "the first row that matches wins" true of
    // the code and not only of the table: the row that hides the panel is the
    // same row that will not let a toggle open it.
    for (const phase of ['Lobby', 'Loading', 'InProgress', 'GameOver', '']) {
      expect(joinPanelAction(phase, true)).toBe('hide');
      expect(joinPanelSuppressed(true)).toBe(true);
    }
    // …and no other 'hide' refuses: Loading and GameOver take the panel away
    // and an operator can still put it back.
    expect(joinPanelAction('Loading', false)).toBe('hide');
    expect(joinPanelSuppressed(false)).toBe(false);
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

// The hint's second field is a TONE — a class name — rather than a colour
// (issue #1358). This module is pure and cannot see a stylesheet, so the three
// hexes it used to return were a slice of the palette that did not follow
// #1357's retint: the lobby was painting pre-graphite navy greys onto a
// graphite surface. What it decides is whether the line is live; what live
// looks like is gui/host-lobby.css's business.
describe('hostLobbyViewModel — status hint', () => {
  it('launching hint wins during countdown', () => {
    const vm = hostLobbyViewModel(payload({ countdown_secs: 3 }), '');
    expect(vm.hint).toEqual({ id: 'server.hint_launching', params: { secs: 3 }, tone: 'live' });
  });

  it('waiting-for-players when nobody has joined', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 0 }), '');
    expect(vm.hint).toEqual({ id: 'server.waiting_players', params: {}, tone: '' });
  });

  it('all-ready hint when the crew is ready', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 1, all_ready: true }), '');
    expect(vm.hint).toEqual({ id: 'client.status_all_ready', params: {}, tone: 'live' });
  });

  it('waiting-for-ready otherwise', () => {
    const vm = hostLobbyViewModel(payload({ crew_count: 1, all_ready: false }), '');
    expect(vm.hint).toEqual({ id: 'server.waiting_ready', params: {}, tone: '' });
  });

  it('names no colour at all, so a retint reaches the lobby without touching this file', () => {
    for (const p of [
      payload({ countdown_secs: 3 }),
      payload({ crew_count: 0 }),
      payload({ crew_count: 1, all_ready: true }),
      payload({ crew_count: 1, all_ready: false }),
    ]) {
      expect(hostLobbyViewModel(p, '').hint.color).toBeUndefined();
    }
  });
});

describe('hostLobbyViewModel — AI-only launch button', () => {
  it('visible only when no crew and no spectators', () => {
    expect(hostLobbyViewModel(payload(), '').aiLaunchVisible).toBe(true);
    expect(hostLobbyViewModel(payload({ crew_count: 1 }), '').aiLaunchVisible).toBe(false);
    expect(hostLobbyViewModel(payload({ spectators: ['Zed'] }), '').aiLaunchVisible).toBe(false);
  });
});

// ── the bridge monitor row (issue #1330) ────────────────────────────────────
//
// The row is the native host's, and the rule that keeps it out of the browser
// host's way is a rule about DATA rather than about which page is asking: no
// monitor roster, no row. So these tests reach it the way both surfaces do —
// through `hostLobbyViewModel`'s third argument — and check the sub-function
// directly where the claim is about the row alone.

const monitor = (overrides = {}) => ({
  identity: 'BRAVIA@3840x2160',
  name: 'BRAVIA',
  width: 3840,
  height: 2160,
  primary: true,
  viewscreen: true,
  stations: [],
  ...overrides,
});

const secondMonitor = (overrides = {}) => monitor({
  identity: 'BenQ EX@1920x1080',
  name: 'BenQ EX',
  width: 1920,
  height: 1080,
  primary: false,
  viewscreen: false,
  ...overrides,
});

describe('hostLobbyMonitorRow — when there is a row at all', () => {
  it('answers null when nothing reported a monitor, which is every browser host', () => {
    // server.html calls the view model with two arguments. That is the whole
    // of "the row is absent on the web host": there is no page check anywhere.
    expect(hostLobbyViewModel(payload(), '').monitorRow).toBeNull();
    expect(hostLobbyMonitorRow(undefined)).toBeNull();
    expect(hostLobbyMonitorRow(null)).toBeNull();
  });

  it('answers null for a roster that reported no monitors', () => {
    // A row of no buttons is not a row. A native host between boot and winit
    // enumerating its displays is briefly in exactly this state.
    expect(hostLobbyMonitorRow({ monitors: [] })).toBeNull();
    expect(hostLobbyMonitorRow({ monitors: [], notices: [{ id: 'x', params: {} }] })).toBeNull();
  });

  it('reaches the view model when a native host does report one', () => {
    const vm = hostLobbyViewModel(payload(), '', { monitors: [monitor()] });
    expect(vm.monitorRow.buttons.length).toBe(1);
  });
});

describe('hostLobbyMonitorRow — how a monitor is named', () => {
  it('names a display by what the OS called it and its native resolution', () => {
    const row = hostLobbyMonitorRow({ monitors: [monitor()] });
    expect(row.buttons[0].label).toEqual({
      id: 'server.monitor_row.monitor',
      params: { name: 'BRAVIA', w: 3840, h: 2160 },
    });
  });

  it('uses a different string for a display the OS did not name', () => {
    // Not the same sentence with an empty slot: "· 1920×1080" reads as a bug.
    const row = hostLobbyMonitorRow({ monitors: [monitor({ name: null })] });
    expect(row.buttons[0].label).toEqual({
      id: 'server.monitor_row.monitor_unnamed',
      params: { w: 3840, h: 2160 },
    });
  });

  it('carries the identity a press names back, untranslated', () => {
    const row = hostLobbyMonitorRow({ monitors: [monitor(), secondMonitor()] });
    expect(row.buttons.map((b) => b.identity))
      .toEqual(['BRAVIA@3840x2160', 'BenQ EX@1920x1080']);
  });
});

describe('hostLobbyMonitorRow — marking', () => {
  it('marks the viewscreen and the primary in words, not only by position', () => {
    // WCAG 1.4.1: which monitor is showing the shared view is the one fact
    // this row carries, so it must not be legible only as a border colour.
    const row = hostLobbyMonitorRow({ monitors: [monitor(), secondMonitor()] });
    expect(row.buttons[0].marks.map((m) => m.id)).toEqual([
      'server.monitor_row.viewscreen',
      'server.monitor_row.primary',
    ]);
    expect(row.buttons[1].marks).toEqual([]);
    expect(row.buttons[0].viewscreen).toBe(true);
    expect(row.buttons[1].viewscreen).toBe(false);
  });

  it('marks a viewscreen that is not the primary as exactly that and nothing more', () => {
    // The operator moved it. `reconcile` keeps that choice across a re-plug of
    // some other screen, so the row has to be able to say so.
    const row = hostLobbyMonitorRow({
      monitors: [
        monitor({ viewscreen: false }),
        secondMonitor({ viewscreen: true }),
      ],
    });
    expect(row.buttons[0].marks.map((m) => m.id)).toEqual(['server.monitor_row.primary']);
    expect(row.buttons[1].marks.map((m) => m.id)).toEqual(['server.monitor_row.viewscreen']);
  });

  it('shows a single monitor marked, which is the whole inert row', () => {
    const row = hostLobbyMonitorRow({ monitors: [monitor()] });
    expect(row.buttons.length).toBe(1);
    expect(row.buttons[0].viewscreen).toBe(true);
  });

  it('carries the consoles a monitor is holding, which is why a press may be refused', () => {
    const row = hostLobbyMonitorRow({
      monitors: [monitor(), secondMonitor({ stations: ['helm', 'weapons'] })],
    });
    expect(row.buttons[1].stations).toEqual(['helm', 'weapons']);
  });

  it('offers those consoles as a line the button can draw, not only as data', () => {
    // The row promises to say what a screen is holding "up front rather than
    // only after the press". A field nothing renders says nothing.
    const row = hostLobbyMonitorRow({
      monitors: [monitor(), secondMonitor({ stations: ['helm', 'weapons'] })],
    });
    expect(row.buttons[1].occupants).toEqual({
      id: 'server.monitor_row.stations',
      params: { stations: 'helm, weapons' },
    });
  });

  it('says nothing about consoles on a monitor that is holding none', () => {
    const row = hostLobbyMonitorRow({ monitors: [monitor(), secondMonitor()] });
    expect(row.buttons[0].occupants).toBeNull();
    expect(row.buttons[1].occupants).toBeNull();
  });
});

describe('hostLobbyMonitorRow — feedback', () => {
  it('passes each notice through as the id and parameters the host sent', () => {
    // The host composes no sentences (AGENTS.md rule 11): a refusal crosses as
    // a strings.csv id, and the renderer's `t` resolves it.
    const row = hostLobbyMonitorRow({
      monitors: [monitor()],
      notices: [
        { id: 'server.bridge_layout.unknown_monitor', params: { monitor: 'Gone@1920x1080' } },
      ],
    });
    expect(row.notices).toEqual([
      { id: 'server.bridge_layout.unknown_monitor', params: { monitor: 'Gone@1920x1080' } },
    ]);
  });

  it('is empty rather than absent when nothing has happened', () => {
    expect(hostLobbyMonitorRow({ monitors: [monitor()] }).notices).toEqual([]);
  });

  it('defaults a notice with no parameters to an empty set rather than undefined', () => {
    const row = hostLobbyMonitorRow({
      monitors: [monitor()],
      notices: [{ id: 'server.bridge_layout.adopt_no_monitors' }],
    });
    expect(row.notices[0].params).toEqual({});
  });
});

// ── the per-station screen rows (issue #1331) ────────────────────────────────
//
// The row is a `map` over the layout law's own `eligibility` output, and the
// point of these tests is that it stays one: the page must not re-derive WHY a
// screen is not offered, because two implementations of one rule are how a
// greyed button and the refusal a press earns start disagreeing. So every case
// below is fed the law's answer verbatim — `selected` / `eligible` /
// `excluded` plus `is-viewscreen` or `full` — and asserts only on what the row
// DOES with it.

const screen = (identity, choice, excluded) => (
  excluded ? { identity, choice, excluded } : { identity, choice }
);

/** A `BridgeLayoutPayload` for a two-monitor bridge with one station. */
const bridge = (overrides = {}) => ({
  monitors: [monitor(), secondMonitor()],
  stations: [{
    station: 'helm',
    monitors: [
      screen('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
      screen('BenQ EX@1920x1080', 'eligible'),
    ],
  }],
  ...overrides,
});

describe('hostLobbyStationRows — when there is a row at all', () => {
  it('answers null on a host with no monitors, which is every browser host', () => {
    expect(hostLobbyStationRows(null)).toBeNull();
    expect(hostLobbyStationRows({ monitors: [], stations: [] })).toBeNull();
  });

  it('answers null when a bridge reported monitors but no roster', () => {
    // A delivery host, or one still in a world-less lobby: the monitor row is
    // real and there is simply nothing to seat.
    expect(hostLobbyStationRows({ monitors: [monitor()], stations: [] })).toBeNull();
  });

  it('keys its rows by station id, which is what joins them to their cards', () => {
    const rows = hostLobbyStationRows(bridge());
    expect(Object.keys(rows)).toEqual(['helm']);
    expect(rows.helm.station).toBe('helm');
  });
});

describe('hostLobbyStationRows — which screens are offered', () => {
  it('never offers the display showing the viewscreen', () => {
    // The acceptance criterion's "lists exactly the non-viewscreen monitors".
    // Dropped on the LAW's reason, not on a second reading of `viewscreen`.
    const row = hostLobbyStationRows(bridge()).helm;
    expect(row.buttons.map((b) => b.identity)).toEqual(['BenQ EX@1920x1080']);
  });

  it('keeps a full screen, greyed, with the reason beside it', () => {
    // Different from a viewscreen: the operator can free a slot here, and a
    // button that vanished would read as a monitor that had gone.
    const row = hostLobbyStationRows(bridge({
      stations: [{
        station: 'helm',
        monitors: [
          screen('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
          screen('BenQ EX@1920x1080', 'excluded', 'full'),
        ],
      }],
    })).helm;
    expect(row.buttons.length).toBe(1);
    expect(row.buttons[0].disabled).toBe(true);
    expect(row.buttons[0].reason).toEqual({ id: 'server.station_row.full', params: {} });
  });

  it('names a screen exactly as the monitor row names it', () => {
    // One display, one sentence, whichever row it appears in.
    const row = hostLobbyStationRows(bridge()).helm;
    expect(row.buttons[0].label).toEqual({
      id: 'server.monitor_row.monitor',
      params: { name: 'BenQ EX', w: 1920, h: 1080 },
    });
  });

  it('falls back to the unnamed sentence for a display the OS did not name', () => {
    const row = hostLobbyStationRows(bridge({
      monitors: [monitor(), secondMonitor({ name: undefined })],
    })).helm;
    expect(row.buttons[0].label.id).toBe('server.monitor_row.monitor_unnamed');
  });

  it('skips a screen the monitor row is not drawing', () => {
    // The host already omits a display the layout knows and nothing reported;
    // offering a station a button with no name and no size would be half a
    // button, which is worse than none.
    const row = hostLobbyStationRows(bridge({ monitors: [monitor()] })).helm;
    expect(row.buttons).toEqual([]);
  });
});

describe('hostLobbyStationRows — which state the row is in', () => {
  it('marks the screen a console is open on and says where it is', () => {
    const row = hostLobbyStationRows(bridge({
      stations: [{
        station: 'helm',
        assigned_to: 'BenQ EX@1920x1080',
        monitors: [
          screen('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
          screen('BenQ EX@1920x1080', 'selected'),
        ],
      }],
    })).helm;
    expect(row.assigned).toBe('BenQ EX@1920x1080');
    expect(row.buttons[0].selected).toBe(true);
    expect(row.off.selected).toBe(false);
  });

  it('selects the off state when no console is open', () => {
    // Which state the row is in has to be legible without comparing the
    // others, so "closed" is a pressed button rather than an absence.
    const row = hostLobbyStationRows(bridge()).helm;
    expect(row.assigned).toBeNull();
    expect(row.off.selected).toBe(true);
    expect(row.buttons.every((b) => !b.selected)).toBe(true);
  });

  it('offers a message instead of an empty strip on a one-screen bridge', () => {
    // The single-monitor acceptance criterion: the only display is the
    // viewscreen, so there is nowhere for a console — and nothing to turn off.
    const row = hostLobbyStationRows({
      monitors: [monitor()],
      stations: [{
        station: 'helm',
        monitors: [screen('BRAVIA@3840x2160', 'excluded', 'is-viewscreen')],
      }],
    }).helm;
    expect(row.buttons).toEqual([]);
    expect(row.off).toBeNull();
    expect(row.message).toEqual({
      id: 'server.station_row.needs_second_monitor',
      params: {},
    });
  });
});

// ── two consoles per screen, and the greying (issue #1332) ───────────────────
//
// Same discipline as above and it is the whole acceptance criterion: the page
// never works out for itself whether a screen is full. Every case here feeds the
// law's verdict verbatim and checks only what the row DOES with it — including
// the case the law now answers differently, a screen filled by a console a
// hand-authored `--profile` opened, which no station row can ever show.

describe('hostLobbyStationRows — a full screen says what is holding it', () => {
  /**
   * A payload whose BenQ is `full`, holding whatever `stations` names — and, in
   * `reserved`, whichever of those the lobby cannot free.
   */
  const fullBenq = (stations, reserved = []) => bridge({
    monitors: [monitor(), secondMonitor({ stations, reserved })],
    stations: [{
      station: 'helm',
      monitors: [
        screen('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
        screen('BenQ EX@1920x1080', 'excluded', 'full'),
      ],
    }],
  });

  it('names the two consoles already on it, so the operator knows what to close', () => {
    // Visible feedback for the refused third: greyed is *that* it cannot be
    // pressed, and this is *why*. The names are the monitor row's own occupant
    // list — the law's `occupants_on` — joined, not a second list built here.
    const row = hostLobbyStationRows(fullBenq(['weapons', 'comms'])).helm;
    expect(row.buttons[0].disabled).toBe(true);
    expect(row.buttons[0].reason).toEqual({
      id: 'server.station_row.full_holding',
      params: { stations: 'weapons, comms' },
    });
  });

  it('names a console a hand-authored profile opened, which has no row of its own', () => {
    // The case that would otherwise grey a screen for no visible reason: a
    // `--pane` participant is a console on the glass and fills a slot, but is
    // not on the roster, so no station card mentions them anywhere.
    const row = hostLobbyStationRows(fullBenq(['Ada', 'Grace'], ['Ada', 'Grace'])).helm;
    expect(row.buttons[0].reason.params.stations).toBe('Ada, Grace');
  });

  it('says the slot is not the lobby\'s to free when an authored console holds it', () => {
    // Since issue #1332 a `--pane` console costs the screen a real slot, and
    // there is no control anywhere in this surface that frees one: no station
    // row, no off button, nothing short of restarting the host with different
    // arguments. A greyed screen that only listed the names would be true and
    // useless — the operator reads "close one of these" and finds nothing to
    // close. So the reason says which of them is unreclaimable.
    const row = hostLobbyStationRows(fullBenq(['Ada', 'weapons'], ['Ada'])).helm;
    expect(row.buttons[0].disabled).toBe(true);
    expect(row.buttons[0].reason).toEqual({
      id: 'server.station_row.full_authored',
      params: { stations: 'Ada, weapons', authored: 'Ada' },
    });
  });

  it('renders that reason as copy that agrees with itself for one name and for several', () => {
    // The params alone cannot catch this, and the first version of the copy did
    // not: `{authored} was opened by …` renders "Ada, Grace was opened…", which
    // is a sentence disagreeing with itself in the exact case the marker exists
    // for — a `--profile` that authored two participant panes on one screen. The
    // copy is number-neutral instead, so both readings are correct English and
    // neither needs a plural rule the string table has no way to express.
    const render = (reason) => t(reason.id, reason.params);

    const one = hostLobbyStationRows(fullBenq(['Ada', 'weapons'], ['Ada'])).helm;
    expect(render(one.buttons[0].reason))
      .toBe('[full — Ada, weapons; opened by the host\'s own settings and not closable from here: Ada]');

    const several = hostLobbyStationRows(fullBenq(['Ada', 'Grace'], ['Ada', 'Grace'])).helm;
    expect(render(several.buttons[0].reason))
      .toBe('[full — Ada, Grace; opened by the host\'s own settings and not closable from here: Ada, Grace]');

    // And the sentence really was resolved from the table, rather than the
    // ⟨id⟩ fallback `t()` answers for a row that is not in strings.csv.
    expect(render(several.buttons[0].reason)).not.toContain('⟨');
  });

  it('keeps the ordinary marker when every console on it can be closed from here', () => {
    // The discrimination, from the other side: two seated stations are two
    // buttons the operator can press, so nothing is said about authored
    // consoles and the sentence stays the short one.
    const row = hostLobbyStationRows(fullBenq(['weapons', 'comms'], [])).helm;
    expect(row.buttons[0].reason.id).toBe('server.station_row.full_holding');
  });

  it('falls back to the bare marker when the row was told nothing about it', () => {
    // Not reachable from a live host — a screen cannot be full and empty — but
    // the button must still say something rather than render a blank list.
    const row = hostLobbyStationRows(fullBenq([])).helm;
    expect(row.buttons[0].reason).toEqual({ id: 'server.station_row.full', params: {} });
  });

  it('never greys a screen the law called eligible, whatever it is holding', () => {
    // The one that would break if the page started counting for itself: a
    // screen holding ONE console has a slot free, and the law says so. A row
    // that greyed on "stations is non-empty" would refuse the second console
    // this whole issue exists to allow.
    const row = hostLobbyStationRows(bridge({
      monitors: [monitor(), secondMonitor({ stations: ['weapons'] })],
    })).helm;
    expect(row.buttons[0].disabled).toBe(false);
    expect(row.buttons[0].reason).toBeNull();
  });
});

describe('hostLobbyStationRows — a screen filling and emptying', () => {
  /** Three stations, and a BenQ the law's verdict is supplied for. */
  const threeStations = (benqChoices, holding) => ({
    monitors: [monitor(), secondMonitor({ stations: holding })],
    stations: ['helm', 'weapons', 'comms'].map((station) => ({
      station,
      assigned_to: benqChoices[station] === 'selected' ? 'BenQ EX@1920x1080' : undefined,
      monitors: [
        screen('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
        screen('BenQ EX@1920x1080', benqChoices[station],
          benqChoices[station] === 'excluded' ? 'full' : undefined),
      ],
    })),
  });

  const benqOf = (rows, station) =>
    rows[station].buttons.find((b) => b.identity === 'BenQ EX@1920x1080');

  it('greys a full screen on every other station’s row and on neither occupant’s', () => {
    const rows = hostLobbyStationRows(threeStations(
      { helm: 'selected', weapons: 'selected', comms: 'excluded' },
      ['helm', 'weapons'],
    ));
    expect(benqOf(rows, 'helm').selected).toBe(true);
    expect(benqOf(rows, 'helm').disabled).toBe(false);
    expect(benqOf(rows, 'weapons').selected).toBe(true);
    expect(benqOf(rows, 'comms').disabled).toBe(true);
    expect(benqOf(rows, 'comms').reason.params.stations).toBe('helm, weapons');
  });

  it('un-greys the vacated screen on every other row when a console leaves it', () => {
    // The acceptance criterion, at the row: one console moved away, so the
    // screen has a slot again — and it comes back on EVERY row that was greyed,
    // not only on the row of the station that moved.
    const rows = hostLobbyStationRows(threeStations(
      { helm: 'selected', weapons: 'eligible', comms: 'eligible' },
      ['helm'],
    ));
    for (const station of ['weapons', 'comms']) {
      expect(benqOf(rows, station).disabled).toBe(false);
      expect(benqOf(rows, station).reason).toBeNull();
    }
    expect(benqOf(rows, 'helm').selected).toBe(true);
  });
});

describe('hostLobbyViewModel — a card carries its own screen row', () => {
  it('hangs the row on the card whose station id matches', () => {
    const vm = hostLobbyViewModel(
      payload({ stations: [helmStation({ id: 'helm' })] }),
      '',
      bridge(),
    );
    expect(vm.cards[0].screens.station).toBe('helm');
  });

  it('gives a card the bridge has no row for none at all', () => {
    // A ship whose lobby roster and whose layout roster disagree — an
    // auxiliary station, or a hull swapped under a stale payload. Better no row
    // than somebody else's.
    const vm = hostLobbyViewModel(
      payload({ stations: [helmStation({ id: 'science' })] }),
      '',
      bridge(),
    );
    expect(vm.cards[0].screens).toBeNull();
  });

  it('gives every card none on the browser host, which passes no layout', () => {
    const vm = hostLobbyViewModel(payload({ stations: [helmStation({ id: 'helm' })] }), '');
    expect(vm.cards[0].screens).toBeNull();
  });
});
describe('fleetStartValidationState', () => {
  const selectedAndBootReady = {
    fleetLinked: true,
    wasmReady: true,
    worldLoaded: true,
    bootReady: true,
    selectedHull: { template_path: 'assets/entities/alliance_cruiser.toml' },
    validatedHullPath: 'assets/entities/alliance_cruiser.toml',
  };

  it('holds validation false until Rust reports terminal presentation readiness', () => {
    expect(fleetStartValidationState({
      ...selectedAndBootReady,
      presentationReady: false,
    })).toBe(false);
    expect(fleetStartValidationState({
      ...selectedAndBootReady,
      presentationReady: true,
    })).toBe(true);
  });

  it('does not let presentation readiness replace selection or station validation', () => {
    expect(fleetStartValidationState({
      ...selectedAndBootReady,
      selectedHull: null,
      presentationReady: true,
    })).toBe(false);
    expect(fleetStartValidationState({
      ...selectedAndBootReady,
      validatedHullPath: 'assets/entities/alliance_destroyer.toml',
      presentationReady: true,
    })).toBe(false);
  });

  it('lets an explicit GM peer validate without inventing a local hull', () => {
    expect(fleetStartValidationState({
      role: 'gm',
      fleetLinked: true,
      wasmReady: true,
      worldLoaded: true,
      bootReady: true,
      selectedHull: null,
      validatedHullPath: null,
      presentationReady: true,
    })).toBe(true);
    expect(fleetStartValidationState({
      role: 'gm',
      fleetLinked: true,
      wasmReady: true,
      worldLoaded: true,
      bootReady: true,
      presentationReady: false,
    })).toBe(false);
  });
});

describe('hostLobbyViewModel GM presence', () => {
  it('ships a labelled host-lobby region with list semantics', () => {
    expect(SERVER_HTML).toContain('id="lobby-gm-group" role="region" aria-labelledby="lobby-gm-heading"');
    expect(SERVER_HTML).toContain('id="lobby-gm-list" class="lobby-rail-section" role="list"');
    expect(SERVER_HTML).toContain('id="fleet-gm-group" role="region" aria-labelledby="fleet-gm-heading"');
    expect(SERVER_HTML).toContain('id="fleet-gms" role="list"');
    expect(SERVER_HTML).toContain("pill.setAttribute('role', 'listitem')");
  });

  it('ships an accessible admitted-GM start region and the fleet/Rust seams', () => {
    expect(SERVER_HTML).toContain('id="gm-start-controls" role="region" aria-labelledby="gm-start-heading" aria-hidden="true"');
    expect(SERVER_HTML).toContain('id="gm-start-policy" role="status" aria-live="polite"');
    expect(SERVER_HTML).toContain('id="gm-start-result" role="status" aria-live="polite"');
    expect(SERVER_HTML.match(/onSimulationRoster: adoptFleetSimulationRoster/g)).toHaveLength(2);
    expect(SERVER_HTML).toContain('onStartPolicy: receiveFleetStartPolicy');
    // Only the mesh owner proposes the pure grant. Members learn the scheduled
    // grant from the owner's authenticated Rust TickFrame.
    expect(SERVER_HTML.match(/onStartGrant: applyFleetStartGrant/g)).toHaveLength(1);
    expect(SERVER_HTML).toContain('onForceResult: receiveFleetForceResult');
    expect(SERVER_HTML).toContain('window.wasm_set_fleet_managed_lobby = wasmBindings.wasm_set_fleet_managed_lobby');
    expect(SERVER_HTML).toContain('window.wasm_fleet_join_status = wasmBindings.wasm_fleet_join_status');
    expect(SERVER_HTML).toContain('window.wasm_leave_fleet = wasmBindings.wasm_leave_fleet');
    expect(SERVER_HTML).toContain('window.wasm_apply_start_grant = wasmBindings.wasm_apply_start_grant');
    expect(SERVER_HTML).toContain('fleetPresentationReady = s.presentation_ready === true');
    expect(SERVER_HTML).toContain('presentationReady: fleetPresentationReady');
    expect(SERVER_HTML).toContain('function validatePlayerHull(templatePath, toml)');
    expect(SERVER_HTML).toContain('if (!status || status.generation !== pending.generation) return false;');
    expect(SERVER_HTML).toContain('if (pollFleetSimulationRoster()) {\n        pollFleetGmJoin();\n        return;');
    expect(SERVER_HTML).toContain('if (fleetSimulationRosterKey) flushFleetStartGrants();');
    expect(SERVER_HTML).toContain("if (refusal !== 'fleet-lobby-input-queue-full')");
    expect(SERVER_HTML).toContain("if (refusal === 'start-grant-queue-full')");
    expect(SERVER_HTML).toContain("canLeave: fleetLobbyPhase === 'Lobby' && !pendingFleetLeave");
    expect(SERVER_HTML).toContain('if (status.status === \'accepted\') {\n          finalizeFleetLeave(leave.reason);');
    expect(SERVER_HTML).toContain("fleetFault = status.reason || 'fleet-leave-refused'");
    expect(SERVER_HTML).toContain('if (adopted && adopted.adopted === true) {\n            pendingFleetSimulationRoster = null;\n            settleFleetSimulationRoster(adopted, true);');
    expect(SERVER_HTML).toContain('attempt.adopted = true;\n        if (!pendingFleetLeave) {');
    expect(SERVER_HTML).toContain('if (pendingFleetSimulationRoster) {\n        paintFleetPanel();\n        return true;');
    expect(SERVER_HTML).toContain('_fleetBootReady = true;\n  publishFleetLobbyState();\n  try { wasm_init();');
  });

  it('keeps the legacy AI launch outside fleet and GM authority', () => {
    expect(SERVER_HTML).toContain("vm.aiLaunchVisible && !fleetHandle && fleetRole !== 'gm'");
    expect(SERVER_HTML).toContain("!fleetHandle && fleetRole !== 'gm' && typeof wasm_force_start === 'function'");
  });

  it('projects connected and disconnected GMs into a distinct equal group', () => {
    const vm = hostLobbyViewModel(payload({
      gms: [
        { id: 'gm-a', name: 'Ada', connected: true, ready: true },
        // A stale ready bit on a disconnected reconnectable row is ignored.
        { id: 'gm-b', name: 'Bo', connected: false, ready: true },
      ],
    }), '');
    expect(vm.gmGroup).toEqual({
      visible: true,
      headingId: 'lobby.gms.heading',
      pills: [
        {
          id: 'gm-a', name: 'Ada', connected: true, ready: true,
          labelId: 'lobby.gms.connected', readinessLabelId: 'lobby.gms.ready',
        },
        {
          id: 'gm-b', name: 'Bo', connected: false, ready: false,
          labelId: 'lobby.gms.disconnected', readinessLabelId: 'lobby.gms.not_ready',
        },
      ],
    });
  });

  it('does not count GMs as crew, spectators or AI-launch blockers', () => {
    const vm = hostLobbyViewModel(payload({
      crew_count: 0,
      max_players: 3,
      spectators: [],
      gms: [{ id: 'gm-a', name: 'Ada', connected: true, ready: false }],
    }), '');
    expect(vm.crew).toEqual({
      count: 0,
      max: 3,
      dots: [false, false, false],
      spectatorTag: { visible: false, count: 0 },
    });
    expect(vm.spectatorPills).toEqual([{ kind: 'empty', id: 'server.no_players', params: {} }]);
    expect(vm.aiLaunchVisible).toBe(true);
  });

  it('hides the GM region when the public roster is empty', () => {
    expect(hostLobbyViewModel(payload(), '').gmGroup).toEqual({
      visible: false,
      headingId: 'lobby.gms.heading',
      pills: [],
    });
  });
});
