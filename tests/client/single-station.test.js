import { describe, it, expect } from 'vitest';
import { shouldHideTabBar } from '../../gui/single-station.js';

describe('shouldHideTabBar', () => {
  it('returns true when in-game with an empty consoles list', () => {
    expect(shouldHideTabBar([], true)).toBe(true);
  });

  it('returns true when in-game with exactly one console (single-station mode)', () => {
    expect(shouldHideTabBar(['helm'], true)).toBe(true);
  });

  it('returns false when in-game with two or more consoles (multi-station)', () => {
    expect(shouldHideTabBar(['helm', 'tactical'], true)).toBe(false);
  });

  it('returns false when NOT in-game with one console (lobby shows bar)', () => {
    expect(shouldHideTabBar(['helm'], false)).toBe(false);
  });

  it('returns false when NOT in-game with no consoles', () => {
    expect(shouldHideTabBar([], false)).toBe(false);
  });

  it('returns true when in-game with null consoles (treated as empty)', () => {
    expect(shouldHideTabBar(null, true)).toBe(true);
  });

  it('returns true when in-game with undefined consoles', () => {
    expect(shouldHideTabBar(undefined, true)).toBe(true);
  });

  it('returns false when NOT in-game with null consoles', () => {
    expect(shouldHideTabBar(null, false)).toBe(false);
  });
});
