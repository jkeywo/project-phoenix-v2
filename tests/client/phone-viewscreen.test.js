/**
 * tests/client/phone-viewscreen.test.js — gui/phone-viewscreen.js
 * (issue #1429, PRD #1418 stories 18-21).
 *
 * Pure-Node tests: `isPhoneViewscreen` answers from `matchMedia`, matches the
 * exact pair of queries the DEVICE_MATRIX phone/tablet/desktop rows resolve
 * to, and degrades honestly (never throws) when the window it is asked about
 * cannot answer.
 */
import { describe, it, expect } from 'vitest';
import {
  PHONE_PORTRAIT_QUERY,
  PHONE_LANDSCAPE_QUERY,
  isPhoneViewscreen,
} from '../../gui/phone-viewscreen.js';
import { DEVICE_MATRIX } from '../fixtures/device-matrix.mjs';

/** A window whose matchMedia answers the real CSS orientation/size queries
 *  for one concrete viewport — close enough to a real UA to exercise the two
 *  queries this module actually asks, without a browser. */
function windowFor(width, height) {
  const orientation = width >= height ? 'landscape' : 'portrait';
  return {
    matchMedia(query) {
      if (query === PHONE_PORTRAIT_QUERY) {
        return { matches: orientation === 'portrait' && width <= 599 };
      }
      if (query === PHONE_LANDSCAPE_QUERY) {
        return { matches: orientation === 'landscape' && height <= 500 };
      }
      return { matches: false };
    },
  };
}

describe('isPhoneViewscreen', () => {
  it('matches every DEVICE_MATRIX row this repo already labels "phone"', () => {
    for (const entry of DEVICE_MATRIX.filter((d) => d.kind === 'phone')) {
      expect(
        isPhoneViewscreen(windowFor(entry.width, entry.height)),
        `${entry.id} (${entry.width}x${entry.height})`,
      ).toBe(true);
    }
  });

  it('the PRD starting regression pair both match: 390x844 portrait and 844x390 landscape', () => {
    expect(isPhoneViewscreen(windowFor(390, 844))).toBe(true);
    expect(isPhoneViewscreen(windowFor(844, 390))).toBe(true);
  });

  it('does not match the desktop/GM landscape rows', () => {
    for (const id of ['desktop-1280x720-gm', 'desktop-1440x900-gm', 'tablet-1280x720-interim-landscape']) {
      const entry = DEVICE_MATRIX.find((d) => d.id === id);
      expect(isPhoneViewscreen(windowFor(entry.width, entry.height)), id).toBe(false);
    }
  });

  it('reads false when matchMedia is missing, throws, or win is null', () => {
    expect(isPhoneViewscreen(null)).toBe(false);
    expect(isPhoneViewscreen({})).toBe(false);
    expect(isPhoneViewscreen({ matchMedia() { throw new Error('nope'); } })).toBe(false);
  });

  it('falls back to the global window when none is supplied, without throwing under Node', () => {
    // jsdom's global `window` (this suite's environment) has no real
    // matchMedia implementation wired to a viewport, so this is a
    // does-not-throw check rather than an assertion on the result.
    expect(() => isPhoneViewscreen()).not.toThrow();
  });
});
