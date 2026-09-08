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
import {
  renderHostLobby,
  MONITOR_BUTTON_ATTR,
  STATION_BUTTON_ATTR,
  STATION_SCREEN_ATTR,
} from '../../gui/host-lobby-render.js';

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
    expect(avatars[0].className).toBe('card-avatar');
    expect(avatars[1].textContent).toBe('TA');
    // The placeholder is dimmed — the one thing that tells an unclaimed seat
    // apart from a claimed one at a glance across a room. A CLASS since issue
    // #1358, not the inline hex this renderer used to write: the colour lives
    // in gui/host-lobby.css, where a retint reaches it.
    expect(avatars[1].className).toBe('card-avatar placeholder');
    expect(avatars[1].style.color).toBe('');
  });

  it('names the holder in words, and names Backfill on a seat nobody holds', () => {
    // The avatar's two letters are an identifier; a room deciding whether to
    // wait for somebody needs the name. And a free Station is not empty — the
    // Backfill rating runs its systems — so the card says which of the two it
    // is rather than leaving a blank line under the rank.
    installLobbyPanel(document);
    render({
      stations: [
        station({ holder_name: 'Ada' }),
        station({ name: 'Tactical', short_code: 'TAC' }),
      ],
    });
    const holders = document.querySelectorAll('#station-grid .card-holder');
    expect(holders[0].textContent).toBe('Ada');
    expect(holders[0].className).toBe('card-holder');
    expect(holders[1].textContent).toBe(t('station.rating.backfill.name'));
    expect(holders[1].className).toBe('card-holder none');
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
    // The tone is a CLASS since issue #1358, so a retint reaches it. Waiting
    // is the quiet state and carries the base class alone; the live states add
    // one, and the element keeps its own class either way — a renderer that
    // wrote only the tone would strip `.lobby-status-hint` off the element the
    // rail's rules are written against.
    expect(hint.className).toBe('lobby-status-hint');
    expect(hint.style.color).toBe('');

    render({ crew_count: 1, stations: [station({ holder_name: 'Ada' })], all_ready: true });
    expect(document.getElementById('lobby-status-hint').textContent)
      .toBe(t('client.status_all_ready'));
    expect(document.getElementById('lobby-status-hint').className)
      .toBe('lobby-status-hint live');
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

// ── the bridge monitor row (issue #1330) ────────────────────────────────────
//
// The row is markup BOTH documents carry — server.html's own `#monitor-row`,
// which the native lobby document is sliced out of — and data only a native
// host supplies. So these render through the same `installLobbyPanel` fixture
// the rest of the file uses, which is what proves the row's ids exist in the
// real page rather than in a stand-in written to match.

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

const benq = (overrides = {}) => monitor({
  identity: 'BenQ EX@1920x1080',
  name: 'BenQ EX',
  width: 1920,
  height: 1080,
  primary: false,
  viewscreen: false,
  ...overrides,
});

/** Render one payload with a monitor roster attached. */
function renderWithLayout(layout, overrides = {}) {
  const vm = hostLobbyViewModel(payload(overrides), '', layout);
  renderHostLobby(document, vm, t);
  return vm;
}

const rowButtons = () =>
  Array.from(document.getElementById('monitor-row-buttons').children);

describe('the bridge monitor row', () => {
  it('is hidden and empty on a host that reported no monitors', () => {
    // The browser host, every frame of its life: server.html carries the
    // markup so there is one lobby, and never fills it.
    installLobbyPanel(document);
    render();
    expect(document.getElementById('monitor-row').style.display).toBe('none');
    expect(rowButtons().length).toBe(0);
  });

  it('draws one button per monitor, named and marked', () => {
    installLobbyPanel(document);
    renderWithLayout({ monitors: [monitor(), benq()] });
    expect(document.getElementById('monitor-row').style.display).toBe('');
    const buttons = rowButtons();
    expect(buttons.length).toBe(2);
    expect(buttons[0].textContent).toContain('BRAVIA');
    expect(buttons[0].textContent).toContain('3840');
    expect(buttons[0].textContent).toContain(t('server.monitor_row.viewscreen'));
    expect(buttons[1].textContent).toContain('BenQ EX');
    expect(buttons[1].textContent).not.toContain(t('server.monitor_row.viewscreen'));
  });

  it('resolves every string through the table rather than showing an id', () => {
    // `t()` answers ⟨id⟩ for a row that is not in strings.csv, so this is the
    // check that the ids the view model names actually exist.
    installLobbyPanel(document);
    renderWithLayout({
      monitors: [monitor(), benq({ name: null })],
      notices: [
        { id: 'server.bridge_layout.unknown_monitor', params: { monitor: 'Gone@1920x1080' } },
      ],
    });
    const text = document.getElementById('monitor-row').textContent;
    expect(text).not.toContain('⟨');
  });

  it('carries each display identity in the attribute the press half reads', () => {
    // The button text is localised and elided; the identity has to reach the
    // host byte-for-byte or the layout law refuses it as an unknown monitor.
    installLobbyPanel(document);
    renderWithLayout({ monitors: [monitor(), benq()] });
    expect(rowButtons().map((b) => b.getAttribute(MONITOR_BUTTON_ATTR)))
      .toEqual(['BRAVIA@3840x2160', 'BenQ EX@1920x1080']);
  });

  it('draws what a monitor is already holding on the button itself', () => {
    // The payload has carried `stations` since the row landed and nothing
    // rendered it, so the row's own promise — say why a press would be refused
    // BEFORE it is pressed — was not kept. A press onto a screen holding a
    // console comes back as a refusal, never as a move.
    installLobbyPanel(document);
    renderWithLayout({
      monitors: [monitor(), benq({ stations: ['helm', 'weapons'] })],
    });
    const buttons = rowButtons();
    const held = buttons[1].querySelector('.monitor-button-stations');
    expect(held).not.toBeNull();
    expect(held.textContent).toContain('helm, weapons');
    expect(held.textContent).not.toContain('⟨');
    expect(buttons[0].querySelector('.monitor-button-stations')).toBeNull();
  });

  it('says which monitor is the viewscreen in a way a screen reader gets too', () => {
    installLobbyPanel(document);
    renderWithLayout({ monitors: [monitor(), benq()] });
    const buttons = rowButtons();
    expect(buttons[0].getAttribute('aria-pressed')).toBe('true');
    expect(buttons[1].getAttribute('aria-pressed')).toBe('false');
    expect(buttons[0].className).toContain('viewscreen');
    expect(buttons[1].className).not.toContain('viewscreen');
  });

  it('builds real buttons, so a keyboard operates the row without any help', () => {
    installLobbyPanel(document);
    renderWithLayout({ monitors: [monitor(), benq()] });
    for (const b of rowButtons()) {
      expect(b.tagName).toBe('BUTTON');
      // `type="button"` and not the default submit: the lobby panel is inside
      // a page that may grow a form, and a submit would reload it.
      expect(b.type).toBe('button');
    }
  });

  it('replaces the buttons on a re-render rather than accumulating them', () => {
    // The press listener is delegated off the container precisely because
    // these do not survive a render.
    installLobbyPanel(document);
    renderWithLayout({ monitors: [monitor(), benq()] });
    renderWithLayout({ monitors: [monitor(), benq()] });
    expect(rowButtons().length).toBe(2);
  });

  it('moves the mark when the viewscreen moves, without rebuilding the lobby', () => {
    installLobbyPanel(document);
    renderWithLayout({ monitors: [monitor(), benq()] });
    renderWithLayout({
      monitors: [monitor({ viewscreen: false }), benq({ viewscreen: true })],
    });
    const buttons = rowButtons();
    expect(buttons[0].getAttribute('aria-pressed')).toBe('false');
    expect(buttons[1].getAttribute('aria-pressed')).toBe('true');
    expect(buttons[1].textContent).toContain(t('server.monitor_row.viewscreen'));
  });

  it('shows a refusal as visible feedback, resolved from its id and parameters', () => {
    // "The lobby never silently ignores me": a press that changed nothing with
    // a clean log is indistinguishable from a broken button.
    installLobbyPanel(document);
    renderWithLayout({
      monitors: [monitor(), benq()],
      notices: [{
        id: 'server.bridge_layout.viewscreen_holds_stations',
        params: { monitor: 'BenQ EX@1920x1080', stations: 'helm, weapons' },
      }],
    });
    const notice = document.getElementById('monitor-row-notice');
    expect(notice.children.length).toBe(1);
    expect(notice.textContent).toContain('BenQ EX@1920x1080');
    expect(notice.textContent).toContain('helm, weapons');
  });

  it('clears the last refusal when the next render carries none', () => {
    installLobbyPanel(document);
    renderWithLayout({
      monitors: [monitor()],
      notices: [{ id: 'server.bridge_layout.unknown_monitor', params: { monitor: 'Gone@1x1' } }],
    });
    renderWithLayout({ monitors: [monitor()] });
    expect(document.getElementById('monitor-row-notice').children.length).toBe(0);
  });

  it('renders a roster change note as its own line beside any cause', () => {
    installLobbyPanel(document);
    renderWithLayout({
      monitors: [monitor()],
      notices: [
        {
          id: 'server.bridge_layout.adopt_viewscreen_gone',
          params: { monitor: 'BenQ EX@1920x1080', replacement: 'BRAVIA@3840x2160' },
        },
        { id: 'server.bridge_layout.adopt_no_monitors', params: { count: '2' } },
      ],
    });
    expect(document.getElementById('monitor-row-notice').children.length).toBe(2);
  });

  it('renders the rest of the lobby on a document with no monitor row at all', () => {
    // The same guarded-write property the AI-launch button has: one renderer,
    // two documents, and a missing element is a branch that does nothing.
    installLobbyPanel(document);
    document.getElementById('monitor-row').remove();
    renderWithLayout({ monitors: [monitor()] }, { scenario_title: 'Combat Test' });
    expect(document.getElementById('lobby-title').textContent).toBe('Combat Test');
  });
});

// ── the per-station screen rows (issue #1331) ────────────────────────────────
//
// The strip is drawn INSIDE a station card, so what these check is the half no
// view-model test can: that a real `<button>` reaches the real markup carrying
// the two attributes the press half reads, that a full screen is disabled
// rather than merely styled, and that the chosen screen says so to a screen
// reader as well as to an eye.

const screenChoice = (identity, choice, excluded) => (
  excluded ? { identity, choice, excluded } : { identity, choice }
);

/** A layout whose one station may open on the BenQ and nowhere else. */
const stationBridge = (rows) => ({
  monitors: [monitor(), benq()],
  stations: rows || [{
    station: 'helm',
    monitors: [
      screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
      screenChoice('BenQ EX@1920x1080', 'eligible'),
    ],
  }],
});

const screenButtons = () =>
  Array.from(document.querySelectorAll('#station-grid .station-screen-button'));

describe('a station card’s screen row', () => {
  it('draws no strip at all on a host that reported no monitors', () => {
    // The browser host: one renderer, two surfaces, and the row is data-driven
    // rather than page-driven.
    installLobbyPanel(document);
    render({ stations: [station({ id: 'helm' })] });
    expect(document.querySelectorAll('.station-screens').length).toBe(0);
  });

  it('draws an off button and one button per offered screen', () => {
    installLobbyPanel(document);
    renderWithLayout(stationBridge(), { stations: [station({ id: 'helm' })] });
    const buttons = screenButtons();
    expect(buttons.length).toBe(2);
    expect(buttons[0].textContent).toContain(t('server.station_row.off'));
    expect(buttons[1].textContent).toContain('BenQ EX');
    // The viewscreen's own display is never among them.
    expect(buttons.some((b) => b.textContent.includes('BRAVIA'))).toBe(false);
  });

  it('carries the station id and the screen identity the press half reads', () => {
    // Both must reach the host byte-for-byte: the layout law refuses an unknown
    // monitor and an off-roster station alike, and the button text is localised.
    installLobbyPanel(document);
    renderWithLayout(stationBridge(), { stations: [station({ id: 'helm' })] });
    const [off, benqButton] = screenButtons();
    expect(off.getAttribute(STATION_BUTTON_ATTR)).toBe('helm');
    expect(off.getAttribute(STATION_SCREEN_ATTR)).toBe('');
    expect(benqButton.getAttribute(STATION_BUTTON_ATTR)).toBe('helm');
    expect(benqButton.getAttribute(STATION_SCREEN_ATTR)).toBe('BenQ EX@1920x1080');
  });

  it('never carries the attribute the viewscreen row is delegated on', () => {
    // A station button that answered to `data-monitor` would move the shared
    // view instead of opening a console.
    installLobbyPanel(document);
    renderWithLayout(stationBridge(), { stations: [station({ id: 'helm' })] });
    for (const b of screenButtons()) {
      expect(b.hasAttribute(MONITOR_BUTTON_ATTR)).toBe(false);
    }
  });

  it('says which screen is chosen to a screen reader, not only in colour', () => {
    installLobbyPanel(document);
    renderWithLayout(
      stationBridge([{
        station: 'helm',
        assigned_to: 'BenQ EX@1920x1080',
        monitors: [
          screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
          screenChoice('BenQ EX@1920x1080', 'selected'),
        ],
      }]),
      { stations: [station({ id: 'helm' })] },
    );
    const [off, benqButton] = screenButtons();
    expect(off.getAttribute('aria-pressed')).toBe('false');
    expect(benqButton.getAttribute('aria-pressed')).toBe('true');
    expect(benqButton.className).toContain('selected');
  });

  it('disables a full screen rather than offering a press that would be refused', () => {
    installLobbyPanel(document);
    renderWithLayout(
      stationBridge([{
        station: 'helm',
        monitors: [
          screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
          screenChoice('BenQ EX@1920x1080', 'excluded', 'full'),
        ],
      }]),
      { stations: [station({ id: 'helm' })] },
    );
    const [off, benqButton] = screenButtons();
    expect(off.disabled).toBe(false);
    expect(benqButton.disabled).toBe(true);
    expect(benqButton.textContent).toContain(t('server.station_row.full'));
  });

  it('names what a full screen is holding, beside the greyed button (issue #1332)', () => {
    // Visible feedback for the refused third console: greyed says the press
    // will not work, and this says which console to close to make room. The
    // names come off the monitor row's own occupant list, so a `--pane`
    // participant — who has no station card anywhere — is named here too.
    installLobbyPanel(document);
    renderWithLayout(
      {
        monitors: [monitor(), benq({ stations: ['Ada', 'weapons'] })],
        stations: [{
          station: 'helm',
          monitors: [
            screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
            screenChoice('BenQ EX@1920x1080', 'excluded', 'full'),
          ],
        }],
      },
      { stations: [station({ id: 'helm' })] },
    );
    const [, benqButton] = screenButtons();
    expect(benqButton.disabled).toBe(true);
    const why = benqButton.querySelector('.station-screen-reason');
    expect(why.textContent).toContain('Ada, weapons');
    expect(why.textContent).not.toContain('⟨');
  });

  it('says why there is no screen on a one-monitor bridge', () => {
    installLobbyPanel(document);
    renderWithLayout(
      {
        monitors: [monitor()],
        stations: [{
          station: 'helm',
          monitors: [screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen')],
        }],
      },
      { stations: [station({ id: 'helm' })] },
    );
    expect(screenButtons().length).toBe(0);
    expect(document.querySelector('.station-screens-message').textContent)
      .toBe(t('server.station_row.needs_second_monitor'));
  });

  it('resolves every string through the table rather than showing an id', () => {
    installLobbyPanel(document);
    renderWithLayout(
      stationBridge([{
        station: 'helm',
        monitors: [
          screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
          screenChoice('BenQ EX@1920x1080', 'excluded', 'full'),
        ],
      }]),
      { stations: [station({ id: 'helm' })] },
    );
    expect(document.querySelector('.station-screens').textContent).not.toContain('⟨');
  });

  it('replaces the strip on a re-render rather than accumulating one per push', () => {
    // The grid is rebuilt wholesale, which is exactly why the press listener is
    // delegated — but a strip appended to a surviving card would stack.
    installLobbyPanel(document);
    renderWithLayout(stationBridge(), { stations: [station({ id: 'helm' })] });
    renderWithLayout(stationBridge(), { stations: [station({ id: 'helm' })] });
    expect(document.querySelectorAll('.station-screens').length).toBe(1);
    expect(screenButtons().length).toBe(2);
  });
});

it('retains an actionable Off control and explanation when an assigned screen is unavailable', () => {
  installLobbyPanel(document);
  renderWithLayout({
    monitors: [monitor()],
    stations: [{ station: 'helm', assigned_to: 'Missing@1920x1080', monitors: [
      screenChoice('BRAVIA@3840x2160', 'excluded', 'is-viewscreen'),
    ] }],
  }, { stations: [station({ id: 'helm' })] });
  expect(screenButtons()).toHaveLength(1);
  expect(screenButtons()[0].getAttribute('aria-pressed')).toBe('false');
  expect(screenButtons()[0].disabled).toBe(false);
  expect(document.querySelector('.station-screens-message').textContent)
    .toBe(t('server.station_row.unavailable'));
});
