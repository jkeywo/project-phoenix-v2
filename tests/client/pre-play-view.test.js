/**
 * tests/client/pre-play-view.test.js — the one pre-play surface decision
 * (issue #1359).
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
import { prePlayView, PRE_PLAY_SURFACES } from '../../gui/pre-play-view.js';

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
