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
import { describe, it, expect } from 'vitest';
import { gameOverView } from '../../gui/game-over-view.js';
import { LobbyState } from '../../gui/lobby-state.js';

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
