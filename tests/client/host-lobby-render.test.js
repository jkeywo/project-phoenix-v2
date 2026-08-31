// @vitest-environment jsdom
//
// tests/client/host-lobby-render.test.js — the extracted lobby renderer
// (issue #1325).
//
// gui/host-lobby-render.js is every DOM write that used to be inline in
// server.html's `__updateLobby`, and it is now rendered by TWO surfaces: the
// host page, and the document the native host composites onto its viewscreen
// window. So the thing worth testing is what an inline function could not be:
// that the writes land in the real markup, that a re-render replaces rather
// than accumulates, and that the document with fewer elements in it — the
// native lobby, which carries no AI-launch button — renders everything else
// instead of stopping at the missing one.
//
// The markup is LIFTED OUT OF server.html rather than hand-written here, for
// the same reason server-settings.test.js lifts #debug-dock: the renderer's
// whole contract is thirteen element ids in a particular nest, and a
// hand-written stand-in would keep passing after the page renamed one of them.
// (The Rust half of that same guard is
// `native_host::host_lobby::document`'s `a_lobby_document_assembles_from_the_
// repositorys_own_host_page`.)
import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { t } from '../../gui/strings.js';
import { hostLobbyViewModel } from '../../gui/host-lobby-view.js';
import { renderHostLobby } from '../../gui/host-lobby-render.js';

const SERVER_HTML = fs.readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), '../../server.html'),
  'utf-8',
);

/** server.html's real #lobby-panel subtree, lifted into the test document. */
function installLobbyPanel(doc, { withAiButton = true } = {}) {
  const parsed = new DOMParser().parseFromString(SERVER_HTML, 'text/html');
  const panel = parsed.getElementById('lobby-panel');
  if (!panel) throw new Error('#lobby-panel not found in server.html');
  const node = doc.importNode(panel, true);
  if (!withAiButton) {
    // What the native lobby document does to the same markup: the surface is
    // read-only in this slice, so the one control is stripped.
    node.querySelectorAll('button').forEach((b) => b.remove());
  }
  doc.body.appendChild(node);
  return node;
}

function payload(overrides = {}) {
  return {
    phase: 'Lobby',
    scenario_title: 'Combat Test',
    scenario_body: 'A shakedown run.',
    crew_count: 0,
    max_players: 2,
    all_ready: false,
    stations: [],
    spectators: [],
    countdown_secs: 0,
    ...overrides,
  };
}

const station = (overrides = {}) => ({
  name: 'Helm', short_code: 'HLM', rank: 'Lieutenant',
  holder_name: null, preset_names: [], consoles: [],
  ...overrides,
});

/** Render one payload into the current document. */
function render(overrides = {}, opts, prevPhase = '') {
  const vm = hostLobbyViewModel(payload(overrides), prevPhase);
  renderHostLobby(document, vm, t, opts);
  return vm;
}

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('the lobby panel is shown by the phase, or forced by the native reveal', () => {
  it('shows the panel in the lobby phase and hides it once a mission starts', () => {
    installLobbyPanel(document);
    render({ phase: 'Lobby' });
    expect(document.getElementById('lobby-panel').style.display).toBe('');

    render({ phase: 'InProgress' }, undefined, 'Lobby');
    expect(document.getElementById('lobby-panel').style.display).toBe('none');
  });

  it('forces the chrome back during play when the host reveals the surface', () => {
    // The native host's F9 (native_host::host_lobby::reveal). The phase still
    // says "hide", so without this option the revealed surface would composite
    // an empty page — which is what it does when it has yielded.
    installLobbyPanel(document);
    render({ phase: 'InProgress' }, { revealChrome: true }, 'Lobby');
    expect(document.getElementById('lobby-panel').style.display).toBe('');
  });

  it('leaves the lobby phase showing whatever the reveal option says', () => {
    // In the lobby the chrome is on regardless: the reveal is a play-time
    // control, and the two answers must not fight.
    installLobbyPanel(document);
    render({ phase: 'Lobby' }, { revealChrome: false });
    expect(document.getElementById('lobby-panel').style.display).toBe('');
  });
});

describe('the header', () => {
  it('writes the scenario title and body', () => {
    installLobbyPanel(document);
    render();
    expect(document.getElementById('lobby-title').textContent).toBe('Combat Test');
    expect(document.getElementById('lobby-subtitle').textContent).toBe('A shakedown run.');
  });

  it('falls back to the unknown-scenario string when the world has no title', () => {
    installLobbyPanel(document);
    render({ scenario_title: '' });
    expect(document.getElementById('lobby-title').textContent)
      .toBe(t('server.unknown_scenario'));
  });

  it('writes the crew counter and one dot per seat, filled up to the crew count', () => {
    installLobbyPanel(document);
    render({ crew_count: 1, max_players: 3 });
    expect(document.getElementById('lobby-crew-count').textContent).toBe('1/3');
    const dots = document.getElementById('lobby-crew-dots').children;
    expect(dots.length).toBe(3);
    expect(dots[0].className).toBe('crew-dot filled');
    expect(dots[1].className).toBe('crew-dot');
  });

  it('shows the spectator tag only when somebody is spectating', () => {
    installLobbyPanel(document);
    render();
    expect(document.getElementById('lobby-spectator-tag').style.display).toBe('none');

    render({ spectators: ['Ada', 'Grace'] });
    const tag = document.getElementById('lobby-spectator-tag');
    expect(tag.style.display).toBe('inline');
    expect(tag.textContent).toBe('+2');
  });

  it('marks the ready badge with the class the stylesheet animates', () => {
    // `#lobby-ready-badge.go` is the pulsing state in gui/host-lobby.css; the
    // class is the whole of how that rule is reached.
    installLobbyPanel(document);
    render();
    expect(document.getElementById('lobby-ready-badge').className).toBe('');

    render({ crew_count: 1, all_ready: true });
    const badge = document.getElementById('lobby-ready-badge');
    expect(badge.className).toBe('go');
    expect(badge.textContent).toBe(t('client.all_crew_ready'));
  });

  it('counts the launch down in the badge and the countdown panel together', () => {
    installLobbyPanel(document);
    render({ countdown_secs: 3, all_ready: true });
    const cd = document.getElementById('lobby-countdown');
    expect(cd.textContent).toBe('3');
    expect(cd.style.display).toBe('flex');
    expect(document.getElementById('lobby-ready-badge').textContent)
      .toBe(t('server.launching_in', { secs: 3 }));

    render({ countdown_secs: 0 });
    expect(document.getElementById('lobby-countdown').style.display).toBe('none');
  });
});

describe('the station grid', () => {
  it('builds one card per station, marking the claimed ones', () => {
    installLobbyPanel(document);
    render({
      crew_count: 1,
      max_players: 2,
      stations: [
        station({ name: 'Helm', holder_name: 'Ada', consoles: ['Helm'] }),
        station({ name: 'Tactical', short_code: 'TAC', rank: 'Ensign' }),
      ],
    });
    const cards = document.querySelectorAll('#station-grid .station-card');
    expect(cards.length).toBe(2);
    expect(cards[0].className).toBe('station-card claimed');
    expect(cards[1].className).toBe('station-card');
    expect(cards[0].querySelector('.card-name').textContent).toBe('Helm');
    expect(cards[1].querySelector('.card-rank').textContent).toBe('Ensign');
  });

  it('takes the avatar initials from the holder, or from the short code when free', () => {
    installLobbyPanel(document);
    render({
      stations: [
        station({ holder_name: 'ada' }),
        station({ name: 'Tactical', short_code: 'tac' }),
      ],
    });
    const avatars = document.querySelectorAll('#station-grid .card-avatar');
    expect(avatars[0].textContent).toBe('AD');
    expect(avatars[0].style.color).toBe('');
    expect(avatars[1].textContent).toBe('TA');
    // The placeholder is dimmed — the one thing that tells an unclaimed seat
    // apart from a claimed one at a glance across a room.
    expect(avatars[1].style.color).not.toBe('');
  });

  it('renders console chips and complexity pills from the roster', () => {
    installLobbyPanel(document);
    render({
      stations: [station({ consoles: ['Helm', 'Navigation'], preset_names: ['Low'] })],
    });
    const chips = document.querySelectorAll('#station-grid .console-chip');
    expect([...chips].map((c) => c.textContent)).toEqual(['Helm', 'Navigation']);
    const pill = document.querySelector('#station-grid .complexity-pill');
    expect(pill.className).toBe('complexity-pill low');
    expect(pill.textContent).toBe(t('server.complexity_low'));
  });

  it('replaces the grid on every render rather than appending to it', () => {
    // The lobby is pushed on every change, so an appending renderer would grow
    // a card per frame — the bug an inline `innerHTML = ''` was guarding
    // against, and one that only shows up after the second push.
    installLobbyPanel(document);
    render({ stations: [station(), station({ name: 'Tactical' })] });
    render({ stations: [station()] });
    expect(document.querySelectorAll('#station-grid .station-card').length).toBe(1);
  });
});

describe('the rail', () => {
  it('lists a pill per crewed station and per waiting spectator', () => {
    installLobbyPanel(document);
    render({
      crew_count: 1,
      stations: [station({ holder_name: 'Ada' })],
      spectators: ['Grace'],
    });
    const pills = document.querySelectorAll('#lobby-spectator-list span');
    expect(pills.length).toBe(2);
    expect(pills[0].className).toBe('spectator-pill');
    expect(pills[0].textContent).toBe('Ada · Helm');
    expect(pills[1].className).toBe('spectator-pill waiting');
    expect(pills[1].textContent).toBe(t('server.spectator_waiting', { name: 'Grace' }));
  });

  it('says so when nobody has connected at all', () => {
    installLobbyPanel(document);
    render();
    const pill = document.querySelector('#lobby-spectator-list span');
    expect(pill.className).toBe('spectator-empty');
    expect(pill.textContent).toBe(t('server.no_players'));
  });

  it('writes the status hint and its colour', () => {
    installLobbyPanel(document);
    render();
    const hint = document.getElementById('lobby-status-hint');
    expect(hint.textContent).toBe(t('server.waiting_players'));
    expect(hint.style.color).not.toBe('');

    render({ crew_count: 1, stations: [station({ holder_name: 'Ada' })], all_ready: true });
    expect(document.getElementById('lobby-status-hint').textContent)
      .toBe(t('client.status_all_ready'));
  });
});

describe('one renderer, two documents', () => {
  it('shows the AI-launch button only with nobody connected', () => {
    installLobbyPanel(document);
    render();
    expect(document.getElementById('ai-launch-btn').style.display).toBe('');

    render({ crew_count: 1, stations: [station({ holder_name: 'Ada' })] });
    expect(document.getElementById('ai-launch-btn').style.display).toBe('none');
  });

  it('renders the whole lobby into a document that has no AI-launch button', () => {
    // The native lobby document (src/native_host/host_lobby/document.rs) strips
    // that button: the surface is read-only and selection stays on the CLI. The
    // renderer must not stop at the missing element — everything before AND
    // after it has to land.
    installLobbyPanel(document, { withAiButton: false });
    expect(document.getElementById('ai-launch-btn')).toBeNull();

    render({
      crew_count: 1,
      stations: [station({ holder_name: 'Ada' })],
      spectators: ['Grace'],
    });
    expect(document.getElementById('lobby-title').textContent).toBe('Combat Test');
    expect(document.querySelectorAll('#station-grid .station-card').length).toBe(1);
    expect(document.getElementById('lobby-status-hint').textContent).not.toBe('');
  });

  it('renders the header alone into a document with no station grid', () => {
    // The original inline glue returned early with no #station-grid, and the
    // extraction keeps that: a document carrying only the header renders the
    // header rather than a half-populated rail.
    installLobbyPanel(document);
    document.getElementById('station-grid').remove();
    expect(() => render({ stations: [station()] })).not.toThrow();
    expect(document.getElementById('lobby-title').textContent).toBe('Combat Test');
  });
});
