import { describe, it, expect } from 'vitest';
import { sectionVisibility, isInGame, IN_GAME_PHASES } from '../../gui/phase-toggle.js';

describe('isInGame', () => {
  it('returns false for Lobby phase', () => {
    expect(isInGame('Lobby')).toBe(false);
  });

  it('returns true for InProgress phase', () => {
    expect(isInGame('InProgress')).toBe(true);
  });

  it('returns true for GameOver phase', () => {
    expect(isInGame('GameOver')).toBe(true);
  });

  it('returns false for unknown phase strings (fail-safe to lobby)', () => {
    expect(isInGame('SomeFuturePhase')).toBe(false);
    expect(isInGame('')).toBe(false);
    expect(isInGame(undefined)).toBe(false);
    expect(isInGame(null)).toBe(false);
  });
});

describe('IN_GAME_PHASES', () => {
  it('lists exactly the two in-game phases', () => {
    expect(IN_GAME_PHASES).toEqual(['InProgress', 'GameOver']);
  });

  it('is frozen so consumers cannot mutate the list', () => {
    expect(Object.isFrozen(IN_GAME_PHASES)).toBe(true);
  });
});

describe('sectionVisibility', () => {
  it('shows lobby and hides game/bezel in Lobby phase', () => {
    expect(sectionVisibility('Lobby')).toEqual({
      lobby: true,
      game: false,
      bezel: false,
    });
  });

  it('hides lobby and shows game/bezel in InProgress phase', () => {
    expect(sectionVisibility('InProgress')).toEqual({
      lobby: false,
      game: true,
      bezel: true,
    });
  });

  it('hides lobby and shows game/bezel in GameOver phase', () => {
    expect(sectionVisibility('GameOver')).toEqual({
      lobby: false,
      game: true,
      bezel: true,
    });
  });

  it('defaults to lobby visibility for unknown phases', () => {
    expect(sectionVisibility('SomeFuturePhase')).toEqual({
      lobby: true,
      game: false,
      bezel: false,
    });
    expect(sectionVisibility(undefined)).toEqual({
      lobby: true,
      game: false,
      bezel: false,
    });
  });
});
