// @vitest-environment jsdom
//
/**
 * tests/client/pre-play-view.test.js — the one pre-play surface decision
 * (issue #1359), and the loading surface projected out of it (issue #1368).
 *
 * Four full-screen surfaces can precede play on client.html, and each of them
 * used to toggle its own `display` from its own call site:
 *
 *   #asset-loading            z-index 200   applySideEffect's show/hide-loading
 *   #scenario-picker-overlay  z-index 186   render(), lobbyState.showScenarioPicker()
 *   #waiting-overlay          z-index 185   render(), lobbyState.waitingForScenario
 *   #join-entry               z-index  30   showEntry()/attempt()'s `.open` class
 *
 * Nothing stopped two of them being displayed at once — the z-index order was
 * the whole of the rule. `legacySurfaces` below reproduces those four
 * independent conditions exactly as they were, so the contradiction tests can
 * assert the thing that actually changed: for an input pair that used to put
 * TWO surfaces on the screen, the decision returns exactly ONE.
 *
 * `legacySurfaces` is a MODEL of the old page, written here — so the last
 * describe block reads client.html itself, in the style of
 * tests/client/settings-panel.test.js, and pins the claims that are about the
 * page rather than about the module: that the priority order really is the
 * stacking order a player used to see, that only one place in the page shows
 * or hides these four elements, that the lead lines this module names are the
 * ones in the markup, and that the applier still works when gui/ never loads.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  prePlayView, loadingView, PRE_PLAY_SURFACES, LOADING_SURFACES,
} from '../../gui/pre-play-view.js';
import { renderLoading } from '../../gui/loading-render.js';
import { buildTable } from '../../gui/strings.js';

const repoFile = (rel) =>
  fs.readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), '../..', rel),
    'utf-8',
  );

/** The consumer. Its stacking order and its toggle sites are page facts. */
const CLIENT_HTML = repoFile('client.html');

/** What a consumer shows: the named surface, and nothing else in the list. */
function shownSurfaces(view) {
  return PRE_PLAY_SURFACES.filter((id) => id === view.surface);
}

/**
 * What the four independent call sites decided for themselves before #1359 —
 * every one of them a condition on its own, with no knowledge of the others.
 */
function legacySurfaces(connection, lobby, preloadFraction) {
  const on = [];
  if (preloadFraction !== null && preloadFraction !== undefined) on.push('asset-loading');
  if (lobby.pickingScenario) on.push('scenario-picker-overlay');
  if (lobby.waitingForScenario) on.push('waiting-overlay');
  if (connection.joinPrompt) on.push('join-entry');
  return on;
}

const STATES = ['connecting', 'ready', 'disconnected', 'error'];
const FRACTIONS = [null, 0, 0.42, 1];

/** The full cross product of the three inputs. */
function everyCombination() {
  const rows = [];
  for (const state of STATES) {
    for (const joinPrompt of [false, true]) {
      for (const pickingScenario of [false, true]) {
        for (const waitingForScenario of [false, true]) {
          for (const preloadFraction of FRACTIONS) {
            rows.push({
              connection: { state, joinPrompt },
              lobby: { pickingScenario, waitingForScenario },
              preloadFraction,
            });
          }
        }
      }
    }
  }
  return rows;
}

describe('PRE_PLAY_SURFACES', () => {
  it('names the four pre-play surfaces by element id, highest z-index first', () => {
    expect(PRE_PLAY_SURFACES).toEqual([
      'asset-loading',
      'scenario-picker-overlay',
      'waiting-overlay',
      'join-entry',
    ]);
  });

  it('is frozen so a consumer cannot mutate the list it hides from', () => {
    expect(Object.isFrozen(PRE_PLAY_SURFACES)).toBe(true);
  });

  it('excludes the post-play game-over overlay', () => {
    // #game-over-overlay is the RESULT of a mission, not a stage before one,
    // and gui/game-over-view.js is already the one pure decision behind it.
    expect(PRE_PLAY_SURFACES).not.toContain('game-over-overlay');
  });
});

describe('one surface at a time', () => {
  const rows = everyCombination();

  it('covers all 128 combinations of the three inputs', () => {
    expect(rows).toHaveLength(STATES.length * 2 * 2 * 2 * FRACTIONS.length);
    expect(rows).toHaveLength(128);
  });

  it('never names a surface outside the enumerated set', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      const view = prePlayView(connection, lobby, preloadFraction);
      if (view.surface !== null) expect(PRE_PLAY_SURFACES).toContain(view.surface);
    }
  });

  it('shows at most one surface in every combination', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      const view = prePlayView(connection, lobby, preloadFraction);
      expect(shownSurfaces(view).length).toBeLessThanOrEqual(1);
    }
  });

  it('shows exactly one whenever any input asks for a surface', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      if (legacySurfaces(connection, lobby, preloadFraction).length === 0) continue;
      const view = prePlayView(connection, lobby, preloadFraction);
      // No exceptions, in any of the 128: priority decides WHICH surface, and
      // nothing suppresses a surface an input asked for.
      expect(shownSurfaces(view)).toHaveLength(1);
    }
  });

  it('shows nothing when no input asks for a surface', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      if (legacySurfaces(connection, lobby, preloadFraction).length > 0) continue;
      const view = prePlayView(connection, lobby, preloadFraction);
      expect(view.surface).toBeNull();
      expect(view.headline).toBeNull();
      expect(shownSurfaces(view)).toHaveLength(0);
    }
  });

  it('picks the surface that was on top before, in every combination', () => {
    // No visual change: the surface a player actually saw is the one with the
    // highest z-index, which is the first entry of the legacy list.
    for (const { connection, lobby, preloadFraction } of rows) {
      const legacy = legacySurfaces(connection, lobby, preloadFraction);
      const view = prePlayView(connection, lobby, preloadFraction);
      expect(view.surface).toBe(legacy.length ? legacy[0] : null);
    }
  });
});

describe('a contradictory input pair yields exactly one surface', () => {
  const conn = { state: 'connecting', joinPrompt: false };
  const noLobby = { pickingScenario: false, waitingForScenario: false };

  const cases = [
    {
      name: 'a catalogue arriving while the host is still being waited on',
      connection: conn,
      lobby: { pickingScenario: true, waitingForScenario: true },
      preloadFraction: null,
      expected: 'scenario-picker-overlay',
    },
    {
      name: 'assets pre-caching while the picker is up',
      connection: conn,
      lobby: { pickingScenario: true, waitingForScenario: false },
      preloadFraction: 0.5,
      expected: 'asset-loading',
    },
    {
      name: 'assets pre-caching while the waiting overlay is up',
      connection: conn,
      lobby: { pickingScenario: false, waitingForScenario: true },
      preloadFraction: 0.5,
      expected: 'asset-loading',
    },
    {
      name: 'a refused join under the scenario picker',
      connection: { state: 'error', joinPrompt: true },
      lobby: { pickingScenario: true, waitingForScenario: false },
      preloadFraction: null,
      expected: 'scenario-picker-overlay',
    },
    {
      name: 'a refused join under the waiting overlay',
      connection: { state: 'error', joinPrompt: true },
      lobby: { pickingScenario: false, waitingForScenario: true },
      preloadFraction: null,
      expected: 'waiting-overlay',
    },
    {
      name: 'a refused join under the preload overlay',
      connection: { state: 'disconnected', joinPrompt: true },
      lobby: noLobby,
      preloadFraction: 0.9,
      expected: 'asset-loading',
    },
    {
      name: 'all four at once',
      connection: { state: 'disconnected', joinPrompt: true },
      lobby: { pickingScenario: true, waitingForScenario: true },
      preloadFraction: 0.1,
      expected: 'asset-loading',
    },
  ];

  for (const c of cases) {
    it(`${c.name}: two surfaces before, one now`, () => {
      // The pair really is contradictory: the old per-site toggles would each
      // have displayed their own surface.
      expect(legacySurfaces(c.connection, c.lobby, c.preloadFraction).length)
        .toBeGreaterThanOrEqual(2);
      const view = prePlayView(c.connection, c.lobby, c.preloadFraction);
      expect(shownSurfaces(view)).toEqual([c.expected]);
    });
  }
});

describe('each input on its own', () => {
  const idle = { state: 'connecting', joinPrompt: false };
  const noLobby = { pickingScenario: false, waitingForScenario: false };

  it('raises the preload overlay for a fraction', () => {
    const view = prePlayView(idle, noLobby, 0.25);
    expect(view.surface).toBe('asset-loading');
    expect(view.headline).toEqual({ id: 'client.preparing_scenario', params: {} });
  });

  it('raises the picker while a catalogue is live', () => {
    const view = prePlayView(idle, { pickingScenario: true, waitingForScenario: false }, null);
    expect(view.surface).toBe('scenario-picker-overlay');
    expect(view.headline).toEqual({ id: 'client.select_scenario', params: {} });
  });

  it('raises the waiting overlay while the host is choosing', () => {
    const view = prePlayView(idle, { pickingScenario: false, waitingForScenario: true }, null);
    expect(view.surface).toBe('waiting-overlay');
    expect(view.headline).toEqual({ id: 'client.waiting_scenario', params: {} });
  });

  it('raises the join field while the page is asking for a code', () => {
    const view = prePlayView({ state: 'connecting', joinPrompt: true }, noLobby, null);
    expect(view.surface).toBe('join-entry');
    expect(view.headline).toEqual({ id: 'client.join.title', params: {} });
  });

  it('shows nothing in the lobby, in play, and on a live link', () => {
    for (const state of STATES) {
      expect(prePlayView({ state, joinPrompt: false }, noLobby, null).surface).toBeNull();
    }
  });

  it('raises the join field in every link state, since the page only asks when it needs one', () => {
    // The connection record is passed whole and only `joinPrompt` is read: the
    // page raises the prompt when it has no code to try or the last one came
    // back refused, and the field is its ONLY input for one. A decision that
    // withheld it in some link state would leave showEntry()'s refusal text and
    // its focus on a panel the guest cannot see.
    for (const state of STATES) {
      expect(prePlayView({ state, joinPrompt: true }, noLobby, null).surface).toBe('join-entry');
    }
  });
});

describe('only the asset preload produces a number', () => {
  const rows = everyCombination();

  it('reports measurable progress for the preload and nothing else', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      const view = prePlayView(connection, lobby, preloadFraction);
      expect(view.measurable).toBe(view.surface === 'asset-loading');
      if (view.surface !== 'asset-loading') expect(view.pct).toBeNull();
    }
  });

  it('never puts a number on a surface that has nothing to measure', () => {
    // The states the AC names one by one: a connecting link, a catalogue
    // waiting on a tap, a host still choosing, a guest typing a code.
    const cases = [
      [{ state: 'connecting', joinPrompt: false }, { pickingScenario: false, waitingForScenario: false }],
      [{ state: 'connecting', joinPrompt: false }, { pickingScenario: true, waitingForScenario: false }],
      [{ state: 'connecting', joinPrompt: false }, { pickingScenario: false, waitingForScenario: true }],
      [{ state: 'error', joinPrompt: true }, { pickingScenario: false, waitingForScenario: false }],
    ];
    for (const [connection, lobby] of cases) {
      const view = prePlayView(connection, lobby, null);
      expect(view.measurable).toBe(false);
      expect(view.pct).toBeNull();
    }
  });

  it('turns the fraction into a whole percentage', () => {
    const idle = { state: 'connecting', joinPrompt: false };
    const noLobby = { pickingScenario: false, waitingForScenario: false };
    expect(prePlayView(idle, noLobby, 0).pct).toBe(0);
    expect(prePlayView(idle, noLobby, 0.5).pct).toBe(50);
    expect(prePlayView(idle, noLobby, 0.577).pct).toBe(58);
    expect(prePlayView(idle, noLobby, 1).pct).toBe(100);
  });

  it('round-trips every whole percentage the reducer emits', () => {
    // gui/lobby-state.js rounds LoadingProgress to a whole pct; client.html
    // hands it back as pct/100, so every one of them has to survive the trip.
    const idle = { state: 'connecting', joinPrompt: false };
    const noLobby = { pickingScenario: false, waitingForScenario: false };
    for (let pct = 0; pct <= 100; pct += 1) {
      expect(prePlayView(idle, noLobby, pct / 100).pct).toBe(pct);
    }
  });

  it('treats a zero fraction as loading, not as absent', () => {
    const view = prePlayView(
      { state: 'connecting', joinPrompt: false },
      { pickingScenario: true, waitingForScenario: false },
      0,
    );
    expect(view.surface).toBe('asset-loading');
    expect(view.pct).toBe(0);
  });

  it('clamps a fraction outside [0, 1] rather than reporting it', () => {
    const idle = { state: 'connecting', joinPrompt: false };
    const noLobby = { pickingScenario: false, waitingForScenario: false };
    expect(prePlayView(idle, noLobby, -0.5).pct).toBe(0);
    expect(prePlayView(idle, noLobby, 4).pct).toBe(100);
  });

  it('treats an absent or unreadable fraction as no preload at all', () => {
    const idle = { state: 'connecting', joinPrompt: false };
    const noLobby = { pickingScenario: false, waitingForScenario: false };
    for (const bad of [null, undefined, '', NaN, Infinity, 'soon']) {
      const view = prePlayView(idle, noLobby, bad);
      expect(view.surface).toBeNull();
      expect(view.measurable).toBe(false);
      expect(view.pct).toBeNull();
    }
  });
});

describe('missing inputs', () => {
  it('shows nothing rather than throwing when an input is absent', () => {
    for (const view of [
      prePlayView(null, null, null),
      prePlayView(undefined, undefined, undefined),
      prePlayView({}, {}, null),
    ]) {
      expect(view.surface).toBeNull();
      expect(view.headline).toBeNull();
      expect(view.measurable).toBe(false);
      expect(view.pct).toBeNull();
    }
  });
});

// ── The page that consumes the decision ──────────────────────────────────────
//
// Everything above drives the module on its own, against a model of the old
// page written in this file. These read client.html, so the ACs that are about
// what a player sees are checked against the thing a player loads.
describe('client.html, the one consumer', () => {
  /**
   * client.html's applier, brace-matched from its declaration — so "outside the
   * applier" below means the rest of the page, not a line that happens to sit
   * elsewhere in the same file.
   */
  function functionBody(name) {
    const open = CLIENT_HTML.indexOf(`function ${name}(`);
    expect(open, `client.html has no function ${name}()`).toBeGreaterThan(-1);
    let i = CLIENT_HTML.indexOf('{', open);
    let depth = 0;
    for (; i < CLIENT_HTML.length; i += 1) {
      if (CLIENT_HTML[i] === '{') depth += 1;
      else if (CLIENT_HTML[i] === '}' && (depth -= 1) === 0) break;
    }
    return CLIENT_HTML.slice(open, i + 1);
  }

  const APPLIER = functionBody('applyPrePlaySurfaces');
  const FALLBACK = functionBody('localPrePlayView');

  const zIndexOf = (id) => {
    const m = CLIENT_HTML.match(new RegExp('#' + id + '\\s*\\{[^}]*z-index:\\s*(\\d+)'));
    expect(m, `no z-index rule for #${id} in client.html`).not.toBeNull();
    return Number(m[1]);
  };

  // The "no visual change" guarantee rests on the priority being the stacking
  // order these four already had. That is a fact about the STYLESHEET, so read
  // it from there rather than trusting the comment that says so — a later hand
  // reordering the z-index values has changed what a player sees, and this is
  // the test that says so out loud.
  it('orders the surfaces exactly as the stylesheet stacks them, highest first', () => {
    const stack = PRE_PLAY_SURFACES.map((id) => [id, zIndexOf(id)]);
    for (let i = 1; i < stack.length; i += 1) {
      const [id, z] = stack[i];
      const [above, zAbove] = stack[i - 1];
      expect(z, `#${id} (z ${z}) is not below #${above} (z ${zAbove})`).toBeLessThan(zAbove);
    }
  });

  // "Every toggle site reads that decision instead of setting display itself."
  // Any new site would have to reach for the element first, and the applier
  // reaches for them only through the list — it never names one.
  it('has no second place that reaches for a pre-play surface element', () => {
    for (const id of PRE_PLAY_SURFACES) {
      const lookup = new RegExp("(getElementById|querySelector)\\(\\s*'#?" + id + "'\\s*\\)");
      expect(CLIENT_HTML.match(lookup), `#${id} is fetched by name outside the applier`)
        .toBeNull();
    }
  });

  it('toggles the join field\'s open class in exactly one place, the applier', () => {
    const toggles = CLIENT_HTML.match(/classList\.(add|remove|toggle)\(\s*'open'/g) || [];
    expect(toggles).toHaveLength(1);
    expect(APPLIER).toMatch(/classList\.toggle\('open', shown\)/);
  });

  it('sets display for these surfaces only inside the applier', () => {
    // The belt to the previous test's braces: every `style.display` left in the
    // page is read with the 250 characters in front of it — the room a
    // getElementById and its `if (el)` need — and none of them may be about one
    // of these four. A fifth toggle site fails the build here.
    const outside = CLIENT_HTML.replace(APPLIER, '');
    expect(APPLIER).toMatch(/style\.display = shown \? 'flex' : 'none'/);
    for (const m of outside.matchAll(/\.style\.display/g)) {
      const before = outside.slice(Math.max(0, m.index - 250), m.index);
      for (const id of PRE_PLAY_SURFACES) {
        expect(before, `a display toggle for #${id} outside the applier`)
          .not.toContain(`'${id}'`);
      }
    }
  });

  // The headline half: the markup's own data-i18n is what renders these lines
  // (gui/strings.js applyToDom writes every one of them), so the decision does
  // NOT write them — it only says what they are. This is what keeps the two
  // from drifting: a wrong id in HEADLINES fails here rather than silently
  // matching nothing on the page.
  it('names the lead line each surface actually carries in its markup', () => {
    const raises = {
      'asset-loading': [{ joinPrompt: false }, {}, 0.5],
      'scenario-picker-overlay': [{}, { pickingScenario: true }, null],
      'waiting-overlay': [{}, { waitingForScenario: true }, null],
      'join-entry': [{ joinPrompt: true }, {}, null],
    };
    for (const id of PRE_PLAY_SURFACES) {
      const view = prePlayView(...raises[id]);
      expect(view.surface).toBe(id);
      const open = CLIENT_HTML.indexOf(`<div id="${id}">`);
      expect(open, `no <div id="${id}"> in client.html`).toBeGreaterThan(-1);
      const block = CLIENT_HTML.slice(open, open + 400);
      expect(block, `#${id} does not carry data-i18n="${view.headline.id}"`)
        .toContain(`data-i18n="${view.headline.id}"`);
    }
  });

  it('writes the percentage from the decision, and only when it is measurable', () => {
    expect(APPLIER).toMatch(/if \(view\.measurable\)/);
    expect(APPLIER).toMatch(/asset-loading-pct/);
  });

  // #join-entry is the page's only input for a join code. Before #1359 it was
  // opened from the classic inline script, depending on nothing; routing it
  // through a gui/ module must not make a guest's only way in hostage to that
  // module loading. The applier therefore never returns before applying, and
  // carries the same ladder inline for the module-absent case.
  it('still decides when gui/pre-play-view.js never loads', () => {
    expect(APPLIER).not.toMatch(/return\s+null\s*;/);
    expect(APPLIER).toMatch(/typeof window\.prePlayView === 'function'/);
    expect(APPLIER).toMatch(/: localPrePlayView/);
    expect(APPLIER).toMatch(/window\.PRE_PLAY_SURFACES\s*:\s*PRE_PLAY_SURFACE_IDS/);
  });

  // Not a claim about the text of the ladder but about what it decides: the
  // inline copy is lifted out of client.html and driven through the same 128
  // combinations as the module, plus the fraction values a reducer can hand it.
  it('decides identically to the module in all 128 combinations', () => {
    // eslint-disable-next-line no-new-func
    const inline = new Function('return (' + FALLBACK + ')')();
    const rows = everyCombination();
    for (const bad of ['', NaN, Infinity, 'soon', undefined, -0.5, 4]) {
      rows.push({
        connection: { state: 'connecting', joinPrompt: false },
        lobby: { pickingScenario: false, waitingForScenario: false },
        preloadFraction: bad,
      });
    }
    for (const { connection, lobby, preloadFraction } of rows) {
      const module = prePlayView(connection, lobby, preloadFraction);
      const fallback = inline(connection, lobby, preloadFraction);
      const where = JSON.stringify({ connection, lobby, preloadFraction });
      expect(fallback.surface, where).toBe(module.surface);
      expect(fallback.measurable, where).toBe(module.measurable);
      expect(fallback.pct, where).toBe(module.pct);
    }
  });

  it('falls back to the same list and the same priority order', () => {
    const list = CLIENT_HTML.match(/const PRE_PLAY_SURFACE_IDS = \[([\s\S]*?)\]/);
    expect(list, 'client.html has no inline PRE_PLAY_SURFACE_IDS').not.toBeNull();
    const ids = [...list[1].matchAll(/'([a-z-]+)'/g)].map((m) => m[1]);
    expect(ids).toEqual([...PRE_PLAY_SURFACES]);
    // …and the inline ladder names them in that order too, so a fallback frame
    // shows the same surface the module would have named.
    const ladder = [...FALLBACK.matchAll(/'([a-z-]+)'/g)]
      .map((m) => m[1])
      .filter((id) => PRE_PLAY_SURFACES.includes(id));
    expect(ladder).toEqual([...PRE_PLAY_SURFACES]);
  });
});

// ── The loading surface (issue #1368) ────────────────────────────────────────
//
// Two of the four surfaces above report progress rather than wait on a person,
// and they now wear one treatment: a ring, a lead line, a progress bar, and the
// Session named underneath it. `loadingView` is a PROJECTION of the decision
// above rather than a second reading of its inputs, which is the whole reason
// it takes `view` as its first argument — the tests below drive it with the
// real `prePlayView` output for exactly that reason, so a disagreement between
// the two would have to be constructed rather than merely inherited.

/** The String Table the client actually ships. */
const STRINGS = buildTable(repoFile('assets/strings/strings.csv'));

/** A resolver that shows the id and its params, so both stay assertable. */
const tid = (id, params) => {
  const keys = Object.keys(params || {});
  return keys.length === 0 ? id : `${id}(${keys.map((k) => `${k}=${params[k]}`).join(',')})`;
};

/** The inputs that raise each loading surface, as prePlayView takes them. */
const RAISES = {
  'asset-loading': [{ joinPrompt: false }, {}, 0.62],
  'waiting-overlay': [{ joinPrompt: false }, { waitingForScenario: true }, null],
};

/** The model for one surface, driven through the real decision. */
function loadingFor(surface, context) {
  const view = prePlayView(...RAISES[surface]);
  expect(view.surface).toBe(surface);
  return loadingView(view, context);
}

/** client.html's real markup for one loading surface, in a fresh document. */
function installSurface(surface, { keep = null } = {}) {
  const parsed = new DOMParser().parseFromString(CLIENT_HTML, 'text/html');
  const node = parsed.getElementById(surface);
  expect(node, `no #${surface} in client.html`).not.toBeNull();
  document.body.innerHTML = '';
  const imported = document.importNode(node, true);
  if (keep) {
    // The deliberately INCOMPLETE document: the page paints from the first
    // server message with its modules still arriving, so a half-mounted
    // surface is a real state and every write has to survive it.
    for (const el of [...imported.querySelectorAll('[id]')]) {
      if (!keep.includes(el.id)) el.remove();
    }
  }
  document.body.appendChild(imported);
  return imported;
}

describe('LOADING_SURFACES', () => {
  it('names the two pre-play surfaces that report progress, in the same priority order', () => {
    expect(LOADING_SURFACES).toEqual(['asset-loading', 'waiting-overlay']);
    expect(Object.isFrozen(LOADING_SURFACES)).toBe(true);
  });

  it('is a subset of the pre-play set, so no third surface can appear only here', () => {
    for (const id of LOADING_SURFACES) expect(PRE_PLAY_SURFACES).toContain(id);
  });

  it('excludes the two surfaces that are waiting on a PERSON, not on work', () => {
    // A catalogue waiting on a tap and a guest typing a code are not progress.
    expect(LOADING_SURFACES).not.toContain('scenario-picker-overlay');
    expect(LOADING_SURFACES).not.toContain('join-entry');
  });
});

describe('loadingView is a projection of the one decision', () => {
  const rows = everyCombination();

  it('produces a model exactly when the decision named a loading surface', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      const view = prePlayView(connection, lobby, preloadFraction);
      const model = loadingView(view, { connectionState: connection.state });
      expect(model === null).toBe(!LOADING_SURFACES.includes(view.surface));
      if (model) expect(model.surface).toBe(view.surface);
    }
  });

  it('carries the measurability the decision already settled, never re-deciding it', () => {
    for (const { connection, lobby, preloadFraction } of rows) {
      const view = prePlayView(connection, lobby, preloadFraction);
      const model = loadingView(view, {});
      if (!model) continue;
      expect(model.measurable).toBe(view.measurable);
      expect(model.pct).toBe(view.pct);
      // The AC in one line: a number only where the decision measured one.
      expect(model.bar.indeterminate).toBe(!view.measurable);
    }
  });

  it('renders nothing at all for a surface it does not own', () => {
    expect(loadingView(prePlayView({ joinPrompt: true }, {}, null), {})).toBeNull();
    expect(loadingView(prePlayView({}, { pickingScenario: true }, null), {})).toBeNull();
    expect(loadingView(prePlayView({}, {}, null), {})).toBeNull();
    expect(loadingView(null, null)).toBeNull();
  });
});

describe('only the asset preload puts a number on the bar', () => {
  it('fills the bar to the measured fraction and counts it against its total', () => {
    const model = loadingFor('asset-loading', {});
    expect(model.measurable).toBe(true);
    expect(model.pct).toBe(62);
    expect(model.bar).toEqual({ indeterminate: false, width: '62%' });
    expect(model.ticks.right).toEqual({ id: 'client.loading_of_total', params: { pct: 62 } });
  });

  it('sweeps without a number, and without an inline width, when nothing is measurable', () => {
    const model = loadingFor('waiting-overlay', {});
    expect(model.measurable).toBe(false);
    expect(model.pct).toBeNull();
    expect(model.bar.indeterminate).toBe(true);
    // EMPTY, not '34%': the sweep's width is a stylesheet decision, and it has
    // to become the whole track under reduced motion. An inline width written
    // from script would outrank the rule that does that.
    expect(model.bar.width).toBe('');
    expect(model.ticks.right).toBeNull();
    expect(model.ticks.left).toEqual({ id: 'client.loading_ticks_none', params: {} });
  });
});

describe('a lost link says it is retrying', () => {
  it('says so on every loading surface, in both lost states', () => {
    for (const surface of LOADING_SURFACES) {
      for (const state of ['disconnected', 'error']) {
        const model = loadingFor(surface, { connectionState: state });
        expect(model.retrying, `${surface} in ${state}`).toBe(true);
        expect(model.status).toEqual({ id: 'client.loading_retrying', params: {} });
      }
    }
  });

  it('says what is being waited on while the link is up', () => {
    for (const state of ['connecting', 'ready', undefined]) {
      expect(loadingFor('asset-loading', { connectionState: state }).status)
        .toEqual({ id: 'client.loading_assets', params: {} });
      expect(loadingFor('waiting-overlay', { connectionState: state }).status)
        .toEqual({ id: 'client.loading_waiting_host', params: {} });
    }
    expect(loadingFor('asset-loading', {}).retrying).toBe(false);
  });

  it('changes a LINE and never a surface — the link state still decides nothing', () => {
    // The line #1359 drew, held from the other side: the retry copy is text on
    // a surface that was already up, so the same inputs must still land on the
    // same surface in every link state.
    for (const { connection, lobby, preloadFraction } of everyCombination()) {
      const base = prePlayView({ ...connection, state: 'ready' }, lobby, preloadFraction);
      const lost = prePlayView({ ...connection, state: 'disconnected' }, lobby, preloadFraction);
      expect(lost.surface).toBe(base.surface);
    }
  });
});

describe('the scenario and the ship are named', () => {
  const session = {
    scenarioTitle: 'Combat Test',
    shipClass: 'destroyer',
    hullId: 'AEV-074',
  };

  it('names both, on the surface whose World is already loaded', () => {
    const { context } = loadingFor('asset-loading', session);
    expect(context.visible).toBe(true);
    expect(context.scenario).toEqual({ text: 'Combat Test' });
    expect(context.ship.hull).toBe('AEV-074');
    expect(context.ship.classId).toBe('component.ship_picker.class.destroyer');
    expect(context.ship.lineId).toBe('client.loading_ship');
  });

  it('names NOTHING on the surface that exists because no World is chosen', () => {
    // The stale-mission trap, and it is the DEFAULT content of this surface on
    // the ordinary GameOver -> ReturnToLobby path: `ReturnedToLobby` sets
    // waitingForScenario and clears the game-over rows, but deliberately
    // leaves scenarioTitle and shipConfig standing (gui/lobby-state.js), so
    // the Session it can see still describes the mission that just ENDED.
    // Printing it under "Waiting for host to select a scenario..." is the same
    // invented fact as a fabricated percentage. Emptiness cannot catch it — a
    // stale title is a present, well-formed string — so the entitlement is a
    // column on the treatment, not a guess about the value.
    const { context } = loadingFor('waiting-overlay', session);
    expect(context.visible).toBe(false);
  });

  it('holds that rule for a Session carrying only one of the three fields', () => {
    for (const partial of [
      { scenarioTitle: 'Combat Test' },
      { shipClass: 'destroyer' },
      { hullId: 'AEV-074' },
    ]) {
      expect(loadingFor('waiting-overlay', partial).context.visible).toBe(false);
      expect(loadingFor('asset-loading', partial).context.visible).toBe(true);
    }
  });

  it('names the class alone when the Session carries no hull id', () => {
    const { context } = loadingFor('asset-loading', { shipClass: 'cruiser' });
    expect(context.visible).toBe(true);
    expect(context.ship.lineId).toBe('client.loading_ship_class_only');
    expect(context.ship.classId).toBe('component.ship_picker.class.cruiser');
  });

  it('hides the block outright before Welcome has landed', () => {
    // The other half of the pair: this is the ENTITLED surface with nothing
    // yet to name, where #waiting-overlay above is an unentitled surface with
    // a full Session sitting right there.
    // Not three empty rows inside a bordered box, which reads as a defect —
    // an absent block reads as "not known yet", which is what is true.
    const { context } = loadingFor('asset-loading', {});
    expect(context.visible).toBe(false);
    expect(context.scenario).toBeNull();
    expect(context.ship.classId).toBe('component.ship_picker.class.unknown');
  });
});

describe('every id the loading surface can emit is authored', () => {
  it('has a String Table row for all of them', () => {
    const ids = new Set();
    for (const surface of LOADING_SURFACES) {
      for (const state of ['ready', 'disconnected']) {
        const m = loadingFor(surface, { connectionState: state, shipClass: 'destroyer', hullId: 'X' });
        ids.add(m.status.id);
        ids.add(m.ticks.left.id);
        if (m.ticks.right) ids.add(m.ticks.right.id);
        ids.add(m.context.ship.lineId);
        ids.add(m.context.ship.classId);
      }
      // The class-only spelling, and the unknown-hull fallback rung.
      const bare = loadingFor(surface, {});
      ids.add(bare.context.ship.lineId);
      ids.add(bare.context.ship.classId);
    }
    const missing = [...ids].filter((id) => !STRINGS.get(id));
    expect(missing, 'ids the loading surface emits with no strings.csv row').toEqual([]);
  });

  it('authors the markup ids each loading surface carries, too', () => {
    // The insignia's accessible name is markup rather than model, so it would
    // not be caught above — and an unauthored one renders as its own id.
    const markupIds = new Set();
    for (const surface of LOADING_SURFACES) {
      const node = installSurface(surface);
      for (const el of node.querySelectorAll('[data-i18n]')) {
        markupIds.add(el.getAttribute('data-i18n'));
      }
      for (const el of node.querySelectorAll('[data-i18n-attr]')) {
        for (const pair of el.getAttribute('data-i18n-attr').split(',')) {
          markupIds.add(pair.split(':')[1]);
        }
      }
    }
    expect(markupIds.has('client.logo_alt')).toBe(true);
    expect([...markupIds].filter((id) => !STRINGS.get(id))).toEqual([]);
  });
});

describe('renderLoading writes the model into a document', () => {
  it('shows the number and fills the bar for the asset preload', () => {
    installSurface('asset-loading');
    renderLoading(document, loadingFor('asset-loading', {
      scenarioTitle: 'Combat Test', shipClass: 'destroyer', hullId: 'AEV-074',
    }), tid);

    expect(document.getElementById('asset-loading-pct').textContent).toBe('62');
    expect(document.getElementById('asset-loading-pct-row').hidden).toBe(false);
    expect(document.getElementById('asset-loading-fill').style.width).toBe('62%');
    expect(document.getElementById('asset-loading-bar').classList.contains('indet')).toBe(false);
    expect(document.getElementById('asset-loading-bar').getAttribute('aria-valuenow')).toBe('62');
    // ...and NOTHING supersedes it. `aria-valuetext` outranks `aria-valuenow`
    // as the announced value, so writing both would publish the percentage to
    // the eye and hide it from assistive tech, which announces the status line
    // forever and never the 62 sitting in the attribute beside it.
    expect(document.getElementById('asset-loading-bar').hasAttribute('aria-valuetext'))
      .toBe(false);
    expect(document.getElementById('asset-loading-tick-right').textContent)
      .toBe('client.loading_of_total(pct=62)');
    expect(document.getElementById('asset-loading-ctx').hidden).toBe(false);
    expect(document.getElementById('asset-loading-scenario').textContent).toBe('Combat Test');
    expect(document.getElementById('asset-loading-ship').textContent)
      .toContain('component.ship_picker.class.destroyer');
    expect(document.getElementById('asset-loading-status').textContent)
      .toBe('client.loading_assets');
  });

  it('shows motion and NO number on a surface with nothing to measure', () => {
    installSurface('waiting-overlay');
    renderLoading(document, loadingFor('waiting-overlay', {}), tid);

    const bar = document.getElementById('waiting-overlay-bar');
    expect(bar.classList.contains('indet')).toBe(true);
    // Absent, not zero: "0%" is the invented number in another form, and an
    // absent aria-valuenow is what tells a screen reader the value is unknown.
    expect(bar.hasAttribute('aria-valuenow')).toBe(false);
    // The converse of the determinate case: with no number to announce, the
    // status line is what the bar is worth saying out loud.
    expect(bar.getAttribute('aria-valuetext')).toBe('client.loading_waiting_host');
    expect(document.getElementById('waiting-overlay-fill').style.width).toBe('');
    expect(document.getElementById('waiting-overlay-tick-right').textContent).toBe('');
    expect(document.getElementById('waiting-overlay-ctx').hidden).toBe(true);
    // This surface has no percentage in its markup at all, and the renderer
    // asked for one anyway without throwing.
    expect(document.getElementById('waiting-overlay-pct')).toBeNull();
  });

  it('keeps the Session block down on the waiting overlay, however full the Session', () => {
    // The rendered half of the stale-mission rule: this is the state a player
    // is actually in after Return to Lobby, and the box that would have named
    // last mission stays shut.
    installSurface('waiting-overlay');
    renderLoading(document, loadingFor('waiting-overlay', {
      scenarioTitle: 'Combat Test', shipClass: 'destroyer', hullId: 'AEV-074',
    }), tid);
    expect(document.getElementById('waiting-overlay-ctx').hidden).toBe(true);
  });

  it('marks the status line while the page is dialling again', () => {
    installSurface('asset-loading');
    renderLoading(document, loadingFor('asset-loading', { connectionState: 'disconnected' }), tid);
    const status = document.getElementById('asset-loading-status');
    expect(status.textContent).toBe('client.loading_retrying');
    expect(status.classList.contains('retrying')).toBe(true);
  });

  it('drops the retry mark again once the link comes back', () => {
    installSurface('asset-loading');
    renderLoading(document, loadingFor('asset-loading', { connectionState: 'error' }), tid);
    renderLoading(document, loadingFor('asset-loading', { connectionState: 'ready' }), tid);
    expect(document.getElementById('asset-loading-status').classList.contains('retrying'))
      .toBe(false);
  });

  it('never writes the lead line, which the markup already owns', () => {
    // #1359's rule: every surface carries its own data-i18n and applyToDom
    // renders it. A renderer that also wrote it would be the second writer of
    // one string, which is the drift HEADLINES exists to prevent.
    installSurface('asset-loading');
    const label = document.querySelector('#asset-loading .ld-label');
    const before = label.textContent;
    renderLoading(document, loadingFor('asset-loading', {}), tid);
    expect(label.textContent).toBe(before);
  });

  it('writes what it can into a half-mounted surface rather than throwing', () => {
    installSurface('asset-loading', { keep: ['asset-loading', 'asset-loading-status'] });
    expect(() => renderLoading(document, loadingFor('asset-loading', {}), tid)).not.toThrow();
    expect(document.getElementById('asset-loading-status').textContent)
      .toBe('client.loading_assets');
  });

  it('does nothing when handed no model', () => {
    installSurface('asset-loading');
    expect(() => renderLoading(document, null, tid)).not.toThrow();
    expect(document.getElementById('asset-loading-status').textContent).toBe('');
  });
});

describe('client.html, the loading surface it draws', () => {
  it('carries every element id the renderer addresses', () => {
    const parts = {
      'asset-loading': [
        'pct-row', 'pct', 'bar', 'fill', 'tick-left', 'tick-right',
        'ctx', 'scenario', 'ship', 'status',
      ],
      // No percentage: nothing about a host still choosing is measurable, and
      // the renderer must survive the difference rather than be told about it.
      'waiting-overlay': [
        'bar', 'fill', 'tick-left', 'tick-right', 'ctx', 'scenario', 'ship', 'status',
      ],
    };
    for (const [surface, ids] of Object.entries(parts)) {
      installSurface(surface);
      for (const part of ids) {
        expect(document.getElementById(`${surface}-${part}`), `#${surface}-${part}`)
          .not.toBeNull();
      }
    }
  });

  it('lets `hidden` actually hide the rows the renderer withholds', () => {
    // The trap this closes, found by looking at the surface rather than at the
    // model: `hidden` is a UA `display: none`, and ANY author `display`
    // outranks it — so the flex column carrying these rows kept the percentage
    // and the Session block on screen holding placeholder text, however
    // carefully the renderer had hidden them.
    expect(CLIENT_HTML).toContain('.ld-stack [hidden] { display: none; }');
  });

  it('gives both loading surfaces the same ring, and stills it without removing it', () => {
    for (const surface of LOADING_SURFACES) {
      const rule = CLIENT_HTML.match(
        new RegExp('#' + surface + ' \\.spinner-ring \\{([^}]*)\\}'),
      );
      expect(rule, `no .spinner-ring rule for #${surface}`).not.toBeNull();
      expect(rule[1]).toMatch(/animation:\s*spin/);
      // Issue #1428 gave decorative motion a control of its own, so the hold
      // hangs off that band; the older attribute keeps its half, gated on the
      // band being absent, for the window before a profile has been applied.
      expect(CLIENT_HTML).toContain(
        `:root[data-decorative-motion="off"] #${surface} .spinner-ring`,
      );
      expect(CLIENT_HTML).toContain(
        ':root[data-reduced-motion="reduce"]:not([data-decorative-motion]) '
        + `#${surface} .spinner-ring`,
      );
    }
  });

  it('rests the fill EMPTY, so an unrendered bar cannot read as full', () => {
    // A block-level fill with no width is width:auto, which is the whole
    // track: before the renderer has run — and on the page's own documented
    // degraded path, where gui/ never loads and applyPrePlaySurfaces() writes
    // only the percentage — the bar paints FULL. That is the invented number
    // in its loudest form, and it is one declaration to make impossible rather
    // than something only script can be trusted to establish.
    expect(CLIENT_HTML).toMatch(/\.ld-fill \{[^}]*width:\s*0/);
  });

  it('gives each progress bar an accessible name', () => {
    // role=progressbar with no name announces as a bare "progress bar".
    for (const surface of LOADING_SURFACES) {
      installSurface(surface);
      const bar = document.getElementById(`${surface}-bar`);
      expect(bar.getAttribute('role')).toBe('progressbar');
      expect(bar.getAttribute('data-i18n-attr')).toMatch(/^aria-label:client\./);
    }
  });

  it('stops the indeterminate sweep when interface animation is off, and keeps the bar', () => {
    const override = CLIENT_HTML.match(
      /:root\[data-decorative-motion="off"\] \.ld-bar\.indet \.ld-fill \{([^}]*)\}/,
    );
    expect(override, 'the sweep has no reduced-motion counterpart').not.toBeNull();
    expect(override[1]).toMatch(/animation:\s*none/);
    // The ring stops but STAYS, and so does the bar: the information the
    // movement carried has to survive the stilling.
    expect(override[1]).not.toMatch(/display\s*:\s*none/);
    expect(override[1]).toMatch(/width:\s*100%/);
  });

  it('switches portrait and landscape in CSS, with no script on the change', () => {
    const rule = '#asset-loading, #waiting-overlay { flex-direction: row; }';
    const at = CLIENT_HTML.indexOf(rule);
    expect(at, 'the loading surfaces never turn into a row').toBeGreaterThan(-1);
    // …and the only thing that turns them is the viewport, so an orientation
    // change relays out without a line of script running.
    expect(CLIENT_HTML.slice(Math.max(0, at - 200), at))
      .toContain('@media (orientation: landscape)');
  });

  it('hands the ONE decision to the loading renderer, and writes the pct itself only if gui/ never loaded', () => {
    const open = CLIENT_HTML.indexOf('function applyPrePlaySurfaces(');
    let i = CLIENT_HTML.indexOf('{', open);
    let depth = 0;
    for (; i < CLIENT_HTML.length; i += 1) {
      if (CLIENT_HTML[i] === '{') depth += 1;
      else if (CLIENT_HTML[i] === '}' && (depth -= 1) === 0) break;
    }
    const applier = CLIENT_HTML.slice(open, i + 1);
    // `view` is the decision this same function just made — the projection is
    // fed from it and never from a second read of the inputs behind it.
    expect(applier).toMatch(/window\.loadingView\(view, loadingContext\(\)\)/);
    expect(applier).toMatch(/window\.loadingRender\.renderLoading\(document, loading, t\)/);
    expect(applier).toMatch(/else if \(view\.measurable\)/);
  });
});
