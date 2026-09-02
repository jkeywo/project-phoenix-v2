/**
 * tests/client/game-over-view.test.js — PRD #1023 module 4, user story 17:
 * "I want the game-over screen to frame the outcome with the scenario's name
 * and result, so that the session lands with an ending rather than a dialog
 * box".
 *
 * The regression worth pinning first is the one that was losing WRITING: the
 * old overlay rendered the hull-death string OR the scenario's authored
 * closing message, never both, so blowing up threw away the world's own
 * account of the defeat.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { gameOverView } from '../../gui/game-over-view.js';
import { LobbyState } from '../../gui/lobby-state.js';
import { localiseTree, setTable, getTable, wireText } from '../../gui/strings.js';

describe('gameOverView — visibility', () => {
  it('is visible only in the GameOver phase', () => {
    expect(gameOverView({ phase: 'GameOver' }).visible).toBe(true);
    expect(gameOverView({ phase: 'InProgress' }).visible).toBe(false);
    expect(gameOverView({ phase: 'Lobby' }).visible).toBe(false);
    expect(gameOverView({}).visible).toBe(false);
  });
});

describe('gameOverView — outcome', () => {
  it('is a defeat when the hull reached zero', () => {
    const vm = gameOverView({ phase: 'GameOver', shipDestroyed: true });
    expect(vm.outcome).toBe('defeat');
    expect(vm.headlineId).toBe('client.game_over_defeat');
  });

  // The wire publishes only { reason }, so a scenario-triggered ending that is
  // not a hull death cannot be classified. Neutral is the honest answer;
  // guessing "victory" from prose is what balance.rs already refused to do.
  it('is neutral when the run ended without a hull death and no declared outcome', () => {
    const vm = gameOverView({ phase: 'GameOver', reason: 'The Harrow fleet withdrew.' });
    expect(vm.outcome).toBe('ended');
    expect(vm.headlineId).toBe('client.game_over_ended');
  });

  it('prefers a declared outcome over the hull-death fallback', () => {
    // Forward compatibility: publishing balance::Outcome on the GameOver
    // message must light this up with no further client work.
    expect(gameOverView({ phase: 'GameOver', outcome: 'victory' }).outcome).toBe('victory');
    expect(gameOverView({ phase: 'GameOver', outcome: 'VICTORY' }).outcome).toBe('victory');
    expect(gameOverView({ phase: 'GameOver', outcome: 'defeat' }).outcome).toBe('defeat');
    // A victory that also destroyed the ship is still the declared victory.
    expect(gameOverView({ phase: 'GameOver', outcome: 'victory', shipDestroyed: true }).outcome)
      .toBe('victory');
  });

  it('ignores an outcome value it does not recognise', () => {
    expect(gameOverView({ phase: 'GameOver', outcome: 'draw' }).outcome).toBe('ended');
    expect(gameOverView({ phase: 'GameOver', outcome: 'draw', shipDestroyed: true }).outcome)
      .toBe('defeat');
  });
});

describe('gameOverView — the frame', () => {
  it('names the scenario', () => {
    const vm = gameOverView({ phase: 'GameOver', scenarioTitle: 'Combat Test' });
    expect(vm.scenarioName).toBe('Combat Test');
  });

  it('has an empty scenario name rather than a placeholder when none is known', () => {
    expect(gameOverView({ phase: 'GameOver' }).scenarioName).toBe('');
  });

  // The bug this module was written for.
  it('keeps the world\'s closing prose on a hull death', () => {
    const vm = gameOverView({
      phase: 'GameOver',
      shipDestroyed: true,
      reason: 'DEFEAT: Starbase Alpha lost.',
      scenarioTitle: 'Combat Test',
    });
    expect(vm.outcome).toBe('defeat');
    expect(vm.bodyText).toBe('DEFEAT: Starbase Alpha lost.');
    expect(vm.bodyId).toBeNull();
  });

  it('falls back to the built-in line only when the world said nothing', () => {
    const vm = gameOverView({ phase: 'GameOver', shipDestroyed: true, reason: '' });
    expect(vm.bodyId).toBe('client.ship_destroyed');
    expect(vm.bodyText).toBe('');
  });

  it('uses the authored reason with no hull death', () => {
    const vm = gameOverView({ phase: 'GameOver', reason: 'All eight waves broken.' });
    expect(vm.bodyId).toBeNull();
    expect(vm.bodyText).toBe('All eight waves broken.');
  });

  it('has no body at all when neither a reason nor a hull death exists', () => {
    const vm = gameOverView({ phase: 'GameOver' });
    expect(vm.bodyId).toBeNull();
    expect(vm.bodyText).toBe('');
  });
});

// ── End to end: the wire message → lobby-state → the framed ending ──────────

/**
 * The half the module could not test before: `ServerMessage::GameOver` now
 * carries `{ reason, outcome }`, so a scenario's declared victory reaches the
 * screen without the view guessing. These feed the real wire shape through the
 * real lobby-state reducer and assert on the frame the overlay renders.
 */
describe('gameOverView — fed from the wire', () => {
  const view = (msg) => {
    const s = new LobbyState();
    s.apply(msg);
    return gameOverView({
      phase: s.phase,
      reason: s.gameOverReason,
      outcome: s.gameOverOutcome,
      scenarioTitle: 'Falling Skyway',
    });
  };

  it('lights up the victory frame from a declared win', () => {
    const vm = view({
      type: 'GameOver',
      data: { reason: 'The skyway held.', outcome: 'victory' },
    });
    expect(vm.visible).toBe(true);
    expect(vm.outcome).toBe('victory');
    expect(vm.headlineId).toBe('client.game_over_victory');
    expect(vm.bodyText).toBe('The skyway held.');
    expect(vm.scenarioName).toBe('Falling Skyway');
  });

  it('lights up the defeat frame from a declared loss, with no hull death', () => {
    const vm = view({
      type: 'GameOver',
      data: { reason: 'The span came down.', outcome: 'defeat' },
    });
    expect(vm.outcome).toBe('defeat');
    expect(vm.headlineId).toBe('client.game_over_defeat');
  });

  // `outcome: null` is what the server writes for an ending that declared no
  // side. It is an answer, not a gap, and the honest frame for it is ENDED.
  it('stays neutral when the ending declared no side', () => {
    const vm = view({ type: 'GameOver', data: { reason: 'The fleet withdrew.', outcome: null } });
    expect(vm.outcome).toBe('ended');
    expect(vm.headlineId).toBe('client.game_over_ended');
  });

  // A host still sending the pre-#1023 shape.
  it('stays neutral when the message carries no outcome field at all', () => {
    const vm = view({ type: 'GameOver', data: { reason: 'The fleet withdrew.' } });
    expect(vm.outcome).toBe('ended');
  });

  it('clears the outcome on the way back to the lobby', () => {
    const s = new LobbyState();
    s.apply({ type: 'GameOver', data: { reason: 'r', outcome: 'victory' } });
    s.apply({ type: 'ReturnedToLobby' });
    expect(s.gameOverOutcome).toBeNull();
    expect(gameOverView({ phase: s.phase, outcome: s.gameOverOutcome }).visible).toBe(false);
  });
});

// ── The post-mission report (issue #1344) ──────────────────────────────────

/**
 * A mission that saved the stricken hauler and lost the skyway is not
 * describable in one word, so a report-bearing ending stops trying: the rows
 * become the result and the win/loss frame steps aside.
 */
const lyraSaved = {
  id: 'lyra',
  heading: 'world.falling_skyway.report.lyra.heading',
  outcome: 'world.falling_skyway.report.lyra.saved',
  state: 'saved',
};
const lyraLost = {
  id: 'lyra',
  heading: 'world.falling_skyway.report.lyra.heading',
  outcome: 'world.falling_skyway.report.lyra.lost',
  state: 'lost',
};
const trafficPartial = {
  id: 'traffic',
  heading: 'world.falling_skyway.report.traffic.heading',
  outcome: 'world.falling_skyway.report.traffic.partial',
  state: 'partial',
};

describe('gameOverView — a report-bearing ending', () => {
  it('is classified reported and headlined as a report, not a verdict', () => {
    const vm = gameOverView({ phase: 'GameOver', report: [lyraSaved] });
    expect(vm.outcome).toBe('reported');
    expect(vm.headlineId).toBe('client.game_over_reported');
  });

  // AC2's catastrophic half. The Lark took the skyway down and the server
  // declared a defeat; the crew still pulled Lyra clear, and the ending has to
  // say so.
  it('outranks a declared defeat and a hull death alike', () => {
    expect(
      gameOverView({ phase: 'GameOver', outcome: 'defeat', report: [lyraSaved] }).outcome,
    ).toBe('reported');
    expect(
      gameOverView({ phase: 'GameOver', shipDestroyed: true, report: [lyraSaved] }).outcome,
    ).toBe('reported');
    expect(
      gameOverView({ phase: 'GameOver', outcome: 'victory', report: [lyraLost] }).outcome,
    ).toBe('reported');
  });

  it('preserves the authored row order', () => {
    const vm = gameOverView({ phase: 'GameOver', report: [lyraSaved, trafficPartial] });
    expect(vm.rows.map((r) => r.id)).toEqual(['lyra', 'traffic']);
  });

  it('hands the caller String Ids to resolve, never prose', () => {
    const [row] = gameOverView({ phase: 'GameOver', report: [lyraSaved] }).rows;
    expect(row.headingId).toBe('world.falling_skyway.report.lyra.heading');
    expect(row.outcomeId).toBe('world.falling_skyway.report.lyra.saved');
    expect(row.state).toBe('saved');
  });

  // AC3, stated where it can fail. The server keeps a signed score per row and
  // a hidden total; neither has any business on a crew's screen, and neither
  // has a field to arrive in.
  it('exposes no score, total, grade or win/loss label', () => {
    const vm = gameOverView({
      phase: 'GameOver',
      outcome: 'defeat',
      report: [{ ...lyraSaved, score: 6 }, { ...trafficPartial, score: -4 }],
    });
    const serialised = JSON.stringify(vm);
    expect(serialised).not.toContain('score');
    expect(serialised).not.toContain('total');
    expect(serialised).not.toContain('grade');
    for (const row of vm.rows) {
      expect(Object.keys(row).sort()).toEqual(['headingId', 'id', 'outcomeId', 'state']);
    }
    // And the frame itself carries no verdict word.
    expect(vm.headlineId).not.toBe('client.game_over_defeat');
    expect(vm.headlineId).not.toBe('client.game_over_victory');
  });

  it("keeps the world's own closing prose beside the rows", () => {
    const vm = gameOverView({
      phase: 'GameOver',
      reason: 'The transfer window has closed.',
      report: [lyraSaved],
    });
    expect(vm.bodyText).toBe('The transfer window has closed.');
    expect(vm.rows).toHaveLength(1);
  });
});

describe('gameOverView — what is NOT a report', () => {
  // AC5. Every scenario that has not authored a report keeps the ending it
  // always had, and the three shapes an absent report can arrive in all mean
  // the same thing.
  it('leaves an unreported ending exactly as it was', () => {
    for (const report of [undefined, [], null, 'nonsense']) {
      expect(gameOverView({ phase: 'GameOver', outcome: 'victory', report }).outcome)
        .toBe('victory');
      expect(gameOverView({ phase: 'GameOver', shipDestroyed: true, report }).outcome)
        .toBe('defeat');
      expect(gameOverView({ phase: 'GameOver', report }).outcome).toBe('ended');
      expect(gameOverView({ phase: 'GameOver', report }).rows).toEqual([]);
    }
  });

  // A row missing either String Id would render as a blank line, which says
  // less than not showing the row.
  it('drops a row that could not render', () => {
    const vm = gameOverView({
      phase: 'GameOver',
      report: [{ id: 'a', heading: 'h.a', state: 'saved' }, { id: 'b', outcome: 'o.b' }, null],
    });
    expect(vm.rows).toEqual([]);
    expect(vm.outcome).toBe('ended');
  });

  // An unknown state still says what happened through its two ids; only the
  // accent is withheld, so a surface styling on it falls back to neutral.
  it('keeps a row whose state it does not recognise, unstyled', () => {
    const vm = gameOverView({
      phase: 'GameOver',
      report: [{ ...lyraSaved, state: 'triumphant' }],
    });
    expect(vm.rows).toHaveLength(1);
    expect(vm.rows[0].state).toBe('');
    expect(vm.outcome).toBe('reported');
  });
});

// ── End to end: the wire message → lobby-state → the reported ending ───────

describe('gameOverView — the report, fed from the wire', () => {
  const view = (msg) => {
    const s = new LobbyState();
    s.apply(msg);
    return gameOverView({
      phase: s.phase,
      reason: s.gameOverReason,
      outcome: s.gameOverOutcome,
      report: s.gameOverReport,
      scenarioTitle: 'Falling Skyway',
    });
  };

  it('lights up the report frame from a real GameOver message', () => {
    const vm = view({
      type: 'GameOver',
      data: {
        reason: 'world.falling_skyway.game_over.lark_collision',
        outcome: 'defeat',
        report: [lyraSaved],
      },
    });
    expect(vm.visible).toBe(true);
    expect(vm.outcome).toBe('reported');
    expect(vm.rows).toHaveLength(1);
    expect(vm.scenarioName).toBe('Falling Skyway');
  });

  // A host still sending the pre-#1344 shape.
  it('stays with the declared frame when the message carries no report field', () => {
    const vm = view({ type: 'GameOver', data: { reason: 'r', outcome: 'victory' } });
    expect(vm.outcome).toBe('victory');
    expect(vm.rows).toEqual([]);
  });

  it('clears the report on the way back to the lobby', () => {
    const s = new LobbyState();
    s.apply({ type: 'GameOver', data: { reason: 'r', outcome: 'defeat', report: [lyraLost] } });
    expect(s.gameOverReport).toHaveLength(1);
    s.apply({ type: 'ReturnedToLobby' });
    expect(s.gameOverReport).toEqual([]);
  });
});

// ── The phone's real ingress: localiseTree runs BEFORE lobby-state ─────────
//
// A phone does not receive String Ids. `gui/rendezvous-transport.js` resolves
// the whole decoded `ServerMessage` through `localiseTree` the moment it
// arrives, so every id the table holds — the report's included, since #1344
// added them to strings.csv — is already prose by the time `LobbyState.apply`
// stores it. The tests above feed `LobbyState` raw ids directly and therefore
// skip that step; this block puts it back, because skipping it is exactly how
// the double-localisation bug (rows rendering as ⟨Lyra Ascending⟩) got in.
describe('the report through the phone ingress', () => {
  const HEADING = 'Lyra Ascending';
  const SAVED = 'Pulled clear of the lane before the band closed.';

  const fixture = () =>
    new Map([
      ['world.falling_skyway.report.lyra.heading', HEADING],
      ['world.falling_skyway.report.lyra.saved', SAVED],
      ['client.game_over_reported', 'MISSION REPORT'],
    ]);

  let saved;
  beforeEach(() => {
    saved = getTable();
    setTable(fixture());
  });
  afterEach(() => setTable(saved));

  /** The row text exactly as client.html's overlay composes it. */
  const rendered = (vm) =>
    vm.rows.map((row) => [wireText(row.headingId), wireText(row.outcomeId)]);

  it('renders the prose, not a doubly-resolved miss', () => {
    const wire = localiseTree({
      type: 'GameOver',
      data: {
        reason: 'world.falling_skyway.game_over.lark_collision',
        outcome: 'defeat',
        report: [lyraSaved],
      },
    });
    // The ingress already did the resolving; the ids are gone from the payload.
    expect(wire.data.report[0].heading).toBe(HEADING);

    const s = new LobbyState();
    s.apply(wire);
    const vm = gameOverView({
      phase: s.phase,
      reason: s.gameOverReason,
      outcome: s.gameOverOutcome,
      report: s.gameOverReport,
      scenarioTitle: 'Falling Skyway',
    });

    expect(vm.outcome).toBe('reported');
    expect(rendered(vm)).toEqual([[HEADING, SAVED]]);
    for (const cell of rendered(vm).flat()) expect(cell).not.toMatch(/^⟨/);
  });

  // The Viewscreen's own resolver (localiseHostPayload) and a host that never
  // learned the ids both hand this surface a still-raw id. One render site has
  // to cover both, so the same call must also resolve.
  it('still resolves a row that arrived unresolved', () => {
    const vm = gameOverView({ phase: 'GameOver', report: [lyraSaved] });
    expect(rendered(vm)).toEqual([[HEADING, SAVED]]);
  });

  // The render site itself, pinned in the page source: `t()` here is the bug,
  // and no view-model test can see which function client.html calls.
  it('is what client.html actually calls on the row fields', () => {
    const page = readFileSync(
      path.join(path.dirname(fileURLToPath(import.meta.url)), '../../client.html'),
      'utf8',
    );
    expect(page).toMatch(/heading\.textContent\s*=\s*wireText\(row\.headingId\)/);
    expect(page).toMatch(/outcome\.textContent\s*=\s*wireText\(row\.outcomeId\)/);
  });
});
