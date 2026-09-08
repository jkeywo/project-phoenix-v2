// @vitest-environment jsdom
//
// tests/client/client-lobby-render.test.js — the extracted client lobby
// renderer (issue #1369).
//
// gui/client-lobby-render.js is every DOM write that used to be inline in
// client.html's `renderLobby`, and the point of lifting it out is that those
// writes become assertable: what used to be provable only by opening a phone
// — that a taken seat's button is disarmed, that the mid-round release swaps
// to a confirm label, that a re-render replaces the roster rather than
// stacking a second copy of it — is now four lines of test each.
//
// The markup is LIFTED OUT OF client.html rather than hand-written here, for
// the reason host-lobby-render.test.js gives for doing the same to
// server.html: the renderer's whole contract is a set of element ids and
// class names in a particular nest, and a hand-written stand-in keeps passing
// after the page renames one of them.
import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { lobbyViewModel } from '../../gui/lobby-view.js';
import {
  renderClientLobby,
  renderClientMods,
  STATION_ROW_ATTR,
  CONSOLE_CHIP_ATTR,
  RATING_BUTTON_ATTR,
} from '../../gui/client-lobby-render.js';

const CLIENT_HTML = fs.readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), '../../client.html'),
  'utf-8',
);

/** client.html's real #lobby-ui subtree, lifted into the test document. */
function installLobby(doc, { keep = null } = {}) {
  const parsed = new DOMParser().parseFromString(CLIENT_HTML, 'text/html');
  const section = parsed.getElementById('lobby-ui');
  if (!section) throw new Error('#lobby-ui not found in client.html');
  const node = doc.importNode(section, true);
  if (keep) {
    // The DELIBERATELY INCOMPLETE document: everything the caller did not name
    // is torn out, standing in for the boot race the lobby actually paints
    // through — client.html loads its modules as ES modules in <head> and
    // paints from the first server message, so a half-mounted shell is a real
    // state and not a hypothetical one.
    for (const el of [...node.querySelectorAll('[id]')]) {
      if (!keep.includes(el.id)) el.remove();
    }
  }
  doc.body.appendChild(node);
  return node;
}

/** A string resolver that shows the id and its params, so both are assertable. */
const t = (id, params) => {
  const keys = Object.keys(params || {});
  return keys.length === 0 ? id : `${id}(${keys.map(k => `${k}=${params[k]}`).join(',')})`;
};

const MY = 'tok-me';

function uiState(overrides = {}) {
  return {
    players: [], gms: [], stations: [], maxPlayers: 0,
    allReady: false, countdownSecs: 0, phase: 'Lobby',
    ...overrides,
  };
}

const station = (overrides = {}) => ({
  id: 'helm', name: 'Helm', short_code: 'HLM', rank: 'Lt',
  holder_name: null, holder_token: null, ratings: ['Std'], description: '',
  ...overrides,
});

/** Build a model from `s` and render it, returning both. */
function render(s, { opts = {}, handlers = {}, token = MY, selected = null } = {}) {
  const vm = lobbyViewModel(s, token, selected, opts);
  renderClientLobby(document, vm, t, handlers);
  return vm;
}

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => [...document.querySelectorAll(sel)];

beforeEach(() => {
  document.body.innerHTML = '';
});

// ── The roster ────────────────────────────────────────────────────────────

describe('the station roster is written from the row model', () => {
  it('draws a row per seat with its glyph, label, rank, console chip and job', () => {
    installLobby(document);
    render(uiState({
      stations: [station({ description: 'Pilot the ship.' })],
    }), { opts: { consoleLabelFor: (id) => 'Console:' + id } });

    const row = $('#station-list .station-row');
    expect(row).not.toBeNull();
    expect(row.getAttribute(STATION_ROW_ATTR)).toBe('helm');
    expect(row.querySelector('.glyph').textContent).toBe('HL');
    expect(row.querySelector('.name').textContent).toBe('Helm');
    expect(row.querySelector('.meta .cons').textContent).toBe('Console:helm');
    // The job is on a FREE row too — reading the seat before claiming it is
    // the whole point of PRD #1023's user story 2.
    expect(row.querySelector('.desc').textContent).toBe('Pilot the ship.');
    expect(row.querySelector('button.claim-btn').disabled).toBe(false);
  });

  it('disarms the seats that cannot be claimed and names who holds one', () => {
    installLobby(document);
    render(uiState({
      players: [{ token: MY, name: 'Ada' }],
      stations: [
        station({ holder_name: 'Bob', holder_token: 'other' }),
        station({ id: 'captain', name: 'Captain' }),
      ],
    }));

    const [taken, free] = $$('#station-list .station-row');
    expect(taken.querySelector('.occupant').textContent).toBe('Bob');
    expect(taken.querySelector('button').disabled).toBe(true);
    expect(taken.querySelector('button').className).toBe('taken-btn');
    expect(free.querySelector('button').disabled).toBe(false);
  });

  it('explains an ineligible seat privately, and never offers its claim', () => {
    installLobby(document);
    render(uiState({ stations: [station()] }), {
      opts: {
        eligibilityFor: () => ({ eligible: false, reason: { functions: ['timed_input'] } }),
      },
    });
    const row = $('#station-list .station-row');
    expect(row.className).toContain('ineligible');
    expect(row.querySelector('.ineligible-reason').textContent)
      .toBe('client.station_ineligible_reason(functions=client.assist_function.timed_input)');
    expect(row.querySelector('button').disabled).toBe(true);
  });

  it('keeps that explanation off a seat somebody already holds', () => {
    // Eligibility is computed for every row, because the anonymous set the
    // page reports to the host is a whole-roster fold — so an ineligible
    // profile makes `eligible: false` true of a TAKEN seat and of the
    // player's OWN seat as well. Neither may say why: the private functional
    // reason belongs to a free seat this player is being turned away from,
    // which is the rule the spectator claim list keeps too.
    installLobby(document);
    render(uiState({
      players: [{ token: MY, name: 'Ada' }],
      stations: [
        station({ holder_name: 'Bob', holder_token: 'other' }),
        station({ id: 'captain', name: 'Captain', holder_name: 'Ada', holder_token: MY }),
      ],
    }), {
      opts: {
        eligibilityFor: () => ({ eligible: false, reason: { functions: ['timed_input'] } }),
      },
    });

    const [taken, mine] = $$('#station-list .station-row');
    expect(taken.className).toBe('station-row taken');
    expect(taken.querySelector('button').className).toBe('taken-btn');
    expect(taken.querySelector('.ineligible-reason')).toBeNull();
    expect(mine.className).toBe('station-row mine');
    expect(mine.querySelector('button').className).toBe('mine-btn');
    expect(mine.querySelector('.ineligible-reason')).toBeNull();
  });

  it('replaces the roster on a re-render rather than accumulating one', () => {
    installLobby(document);
    const s = uiState({ stations: [station(), station({ id: 'captain' })] });
    render(s);
    render(s);
    expect($$('#station-list .station-row')).toHaveLength(2);
  });
});

// ── The six injected actions ──────────────────────────────────────────────

describe('the actions arrive as handlers rather than as closures over the page', () => {
  const seated = (overrides = {}) => uiState({
    players: [{ token: MY, name: 'Ada' }],
    stations: [station({ holder_name: 'Ada', holder_token: MY, ratings: ['Std', 'Simplified'] })],
    ...overrides,
  });

  it('a claim hands the row back, so the caller sends the name the host matches on', () => {
    installLobby(document);
    const claimed = [];
    render(uiState({ stations: [station()] }), { handlers: { claim: (row) => claimed.push(row) } });
    $('#station-list button.claim-btn').click();
    expect(claimed).toHaveLength(1);
    expect(claimed[0].name).toBe('Helm');
  });

  it('the row RELEASE and the detail LEAVE call the same one handler', () => {
    installLobby(document);
    let released = 0;
    render(seated(), { handlers: { release: () => { released += 1; } } });
    $('#station-list button.mine-btn').click();
    $('#detail-panel .detail-release-btn').click();
    expect(released).toBe(2);
  });

  it('a console chip and a complexity button report what was pressed', () => {
    installLobby(document);
    const consoles = [];
    const ratings = [];
    render(seated(), {
      handlers: { selectConsole: (id) => consoles.push(id), selectRating: (r) => ratings.push(r) },
    });
    $(`#detail-panel .chip[${CONSOLE_CHIP_ATTR}="helm"]`).click();
    // Std is the default active rating, so pressing it is a no-op — the same
    // guard the inline code carried, kept because it stops a redundant
    // SetStationRating crossing the wire on every stray tap.
    $(`#detail-panel .rating-btn[${RATING_BUTTON_ATTR}="Std"]`).click();
    $(`#detail-panel .rating-btn[${RATING_BUTTON_ATTR}="Simplified"]`).click();
    expect(consoles).toEqual(['helm']);
    expect(ratings).toEqual(['Simplified']);
  });

  it('ready and spectate send what the view model decided, not a re-derived flag', () => {
    installLobby(document);
    const ready = [];
    const spectator = [];
    render(seated(), {
      handlers: { setReady: (v) => ready.push(v), setSpectator: (v) => spectator.push(v) },
    });
    $('#ready-btn').click();
    $('#spectate-btn').click();
    expect(ready).toEqual([true]);
    expect(spectator).toEqual([true]);
  });

  it('does not stack a listener on the buttons that survive a re-render', () => {
    // #ready-btn and #spectate-btn are in the page's static markup, so they
    // take `onclick =` (which replaces) rather than addEventListener (which
    // would add one more SetReady per paint).
    installLobby(document);
    const ready = [];
    for (let i = 0; i < 3; i += 1) {
      render(seated(), { handlers: { setReady: (v) => ready.push(v) } });
    }
    $('#ready-btn').click();
    expect(ready).toEqual([true]);
  });

  it('renders an inert lobby, rather than throwing, when no handlers are given', () => {
    installLobby(document);
    render(seated());
    expect(() => {
      $('#station-list button.mine-btn').click();
      $('#ready-btn').click();
      $('#spectate-btn').click();
    }).not.toThrow();
  });
});

// ── The labels the model chose ────────────────────────────────────────────

describe('the renderer resolves string ids and never picks one', () => {
  const seated = (overrides = {}) => uiState({
    players: [{ token: MY, name: 'Ada' }],
    stations: [station({ holder_name: 'Ada', holder_token: MY })],
    ...overrides,
  });

  it('shows the resting release labels in the lobby', () => {
    installLobby(document);
    render(seated());
    expect($('#station-list button.mine-btn').textContent).toBe('client.release');
    expect($('#detail-panel .detail-release-btn').textContent).toBe('client.change_station');
  });

  it('swaps BOTH release controls to the confirm label once armed mid-round', () => {
    // #771 AC3/AC4. The swap used to be `inProgress && releaseArmed ? … : …`
    // spelled twice inside the DOM glue; it is one decision in the view model
    // now, and this is what proves it reaches both controls.
    installLobby(document);
    render(seated({ phase: 'InProgress' }), { opts: { releaseArmed: true } });
    expect($('#station-list button.mine-btn').textContent).toBe('client.release_confirm');
    expect($('#detail-panel .detail-release-btn').textContent).toBe('client.release_confirm');
  });

  it('ignores the armed latch in the lobby, where release is immediate', () => {
    installLobby(document);
    render(seated({ phase: 'Lobby' }), { opts: { releaseArmed: true } });
    expect($('#station-list button.mine-btn').textContent).toBe('client.release');
  });

  it('writes the ready button label and class the mode carries', () => {
    installLobby(document);
    render(seated());
    expect($('#ready-btn').textContent).toBe('client.ready');
    expect($('#ready-btn').className).toBe('armed');

    render(seated({ countdownSecs: 7 }));
    expect($('#ready-btn').textContent).toBe('7s');
    expect($('#ready-btn').className).toBe('armed glow');
  });

  it('hides the ready button for a seatless participant', () => {
    installLobby(document);
    render(uiState({ players: [{ token: MY, name: 'Ada' }], stations: [station()] }));
    expect($('#ready-btn').style.display).toBe('none');
  });

  it('resolves the status line with its params', () => {
    installLobby(document);
    render(seated({ countdownSecs: 3 }));
    expect($('#status-line').textContent).toBe('client.status_launching(secs=3)');
  });
});

// ── The header, restyled through this renderer ────────────────────────────

describe('the lobby header is painted from the view model', () => {
  it('writes the crew count and the awaiting badge', () => {
    installLobby(document);
    render(uiState({
      maxPlayers: 3,
      stations: [station({ holder_name: 'Ada' }), station({ id: 'captain' })],
    }));
    expect($('#crew-display').textContent).toBe('1/3');
    expect($('#ready-pill').textContent).toBe('client.awaiting_crew');
    expect($('#ready-pill').className).toBe('');
  });

  it('marks all-ready in words as well as in colour', () => {
    // The badge's `go` class is a colour swap, and a colour alone is not a cue
    // (WCAG 1.4.1). The cue that carries the same fact without colour is the
    // pill's TEXT, which is why the two states resolve different string ids
    // and not just different classes.
    installLobby(document);
    render(uiState({ allReady: true, maxPlayers: 1, stations: [station({ holder_name: 'Ada' })] }));
    expect($('#ready-pill').textContent).toBe('client.all_crew_ready');
    expect($('#ready-pill').className).toBe('go');
  });
});

// ── GM presence ───────────────────────────────────────────────────────────

describe('equal GM peers get their own labelled region', () => {
  it('lists each peer as a listitem carrying its readiness', () => {
    installLobby(document);
    render(uiState({
      gms: [
        { id: 'gm-a', name: 'Morgan', connected: true, ready: true },
        { id: 'gm-b', name: 'Rin', connected: false, ready: true },
      ],
    }));
    expect($('#gm-presence').getAttribute('aria-hidden')).toBe('false');
    const pills = $$('#gm-presence-list .gm-presence-pill');
    expect(pills.map(p => p.getAttribute('role'))).toEqual(['listitem', 'listitem']);
    expect(pills.map(p => p.dataset.ready)).toEqual(['true', 'false']);
    expect(pills[0].textContent).toBe('lobby.gms.connected(name=Morgan) · lobby.gms.ready');
    expect(pills[1].className).toContain('disconnected');
  });

  it('hides the region when no GM is in the session', () => {
    installLobby(document);
    render(uiState());
    expect($('#gm-presence').getAttribute('aria-hidden')).toBe('true');
    expect($$('#gm-presence-list .gm-presence-pill')).toHaveLength(0);
  });
});

// ── The incomplete document ───────────────────────────────────────────────

describe('a partially-mounted lobby paints what it has', () => {
  it('renders the header alone when nothing else is in the document yet', () => {
    installLobby(document, { keep: ['lobby-header', 'ship-info', 'ship-name', 'ship-mission', 'crew-ready', 'crew-display', 'ready-pill'] });
    expect(() => render(uiState({ maxPlayers: 2, stations: [station({ holder_name: 'Ada' })] })))
      .not.toThrow();
    expect($('#crew-display').textContent).toBe('1/2');
    expect($('#station-list')).toBeNull();
  });

  it('renders into an EMPTY document without throwing', () => {
    // The extreme of the boot race: the modules resolved before any lobby
    // markup was parsed. Every write in this module is guarded on its element
    // for this case, which is what lets one renderer serve two documents.
    expect(() => {
      renderClientLobby(document, lobbyViewModel(uiState(), MY, null), t, {});
      renderClientMods(document, [{ name: 'Falling Skyway', version: '1.2' }], t);
    }).not.toThrow();
  });

  it('survives a detail panel that has lost its inner slots', () => {
    installLobby(document);
    $('#detail-panel').innerHTML = '';
    expect(() => render(uiState({
      players: [{ token: MY, name: 'Ada' }],
      stations: [station({ holder_name: 'Ada', holder_token: MY })],
    }))).not.toThrow();
    expect($('#detail-panel').className).toBe('active');
  });
});

// ── The mods list ─────────────────────────────────────────────────────────

describe('the active-mods list is its own entry point', () => {
  it('lists the packs the host applied, with their versions', () => {
    installLobby(document);
    renderClientMods(document, [{ name: 'Falling Skyway', version: '1.2' }, { name: 'Thin Margin' }], t);
    const el = $('#lobby-mods');
    expect(el.getAttribute('aria-hidden')).toBe('false');
    expect(el.querySelector('.mods-active-heading').textContent).toBe('client.mods_active');
    expect($$('#lobby-mods .mods-active-row')).toHaveLength(2);
    expect($('#lobby-mods .mods-active-version').textContent)
      .toBe('client.mods_active_version(version=1.2)');
  });

  it('stays hidden and empty in the base game, so no banner appears', () => {
    installLobby(document);
    renderClientMods(document, [], t);
    expect($('#lobby-mods').getAttribute('aria-hidden')).toBe('true');
    expect($('#lobby-mods').children).toHaveLength(0);
  });
});

describe('claimed station guidance and assigned native screens', () => {
  it('replaces the roster with the shared console guide until the station is released', () => {
    installLobby(document);
    const state = uiState({ players: [{ token: MY }], stations: [station({ holder_token: MY })] });
    render(state);
    expect($('#station-list').style.display).toBe('none');
    expect($('#claimed-station-help').hidden).toBe(false);
    expect($('#claimed-station-help .station-help-section')).not.toBeNull();
    expect($('#detail-panel .detail-release-btn').textContent).toBe('client.change_station');
    render(uiState({ players: [{ token: MY }], stations: [station()] }));
    expect($('#station-list').style.display).toBe('');
    expect($('#claimed-station-help').hidden).toBe(true);
  });

  it('an assigned native screen offers readiness and guide but no release or station switch', () => {
    installLobby(document);
    render(uiState({ players: [{ token: MY }], stations: [station({ holder_token: MY })] }), {
      opts: { assignedStation: 'helm' },
    });
    expect($('#claimed-station-help').hidden).toBe(false);
    expect($('#station-list button')).toBeNull();
    expect($('#detail-panel .detail-release-btn')).toBeNull();
    expect($('#spectate-btn').style.display).toBe('none');
    expect($('#ready-btn').style.display).not.toBe('none');
  });
});
