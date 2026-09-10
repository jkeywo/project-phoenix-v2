/**
 * tests/client/device-matrix.test.js — pins the shape of
 * tests/fixtures/device-matrix.mjs (issue #1421, PRD #1418).
 *
 * This is NOT the acceptance run itself — #1421 is a HITL preparation issue;
 * a human performs the device/browser/room acceptance recorded in
 * docs/acceptance/1421-device-matrix.md. This test only pins the CONTRACT the
 * shared fixture module promises both consumers (tests/client vitest specs
 * and tests/smoke Playwright specs):
 *
 *   - DEVICE_MATRIX entries are well-formed and every one NAMES A SOURCE —
 *     the no-fabricated-device-minimums rule made checkable, not just a
 *     comment;
 *   - TEXT_SCALES/BROWSER_ZOOMS are non-empty ascending numeric lists;
 *   - every DENSE_CONTENT id is a REAL row in the String Table today (via
 *     gui/strings.js `has()`, loaded from the real assets/strings/strings.csv
 *     by tests/client/setup-strings.js) — so dense-content fixtures can never
 *     silently drift onto a deleted or renamed string id.
 */
import { describe, it, expect } from 'vitest';
import { has } from '../../gui/strings.js';
import {
  DEVICE_MATRIX,
  TEXT_SCALES,
  BROWSER_ZOOMS,
  DENSE_CONTENT,
} from '../fixtures/device-matrix.mjs';

const KINDS = new Set(['phone', 'tablet', 'desktop', 'split-pane']);
const ORIENTATIONS = new Set(['portrait', 'landscape']);

describe('DEVICE_MATRIX', () => {
  it('is a non-empty array', () => {
    expect(Array.isArray(DEVICE_MATRIX)).toBe(true);
    expect(DEVICE_MATRIX.length).toBeGreaterThan(0);
  });

  it('every entry has a well-formed id/width/height/kind/orientation', () => {
    for (const entry of DEVICE_MATRIX) {
      expect(typeof entry.id).toBe('string');
      expect(entry.id.length).toBeGreaterThan(0);
      expect(Number.isFinite(entry.width)).toBe(true);
      expect(entry.width).toBeGreaterThan(0);
      expect(Number.isFinite(entry.height)).toBe(true);
      expect(entry.height).toBeGreaterThan(0);
      expect(KINDS.has(entry.kind)).toBe(true);
      expect(ORIENTATIONS.has(entry.orientation)).toBe(true);
    }
  });

  it('every entry names a non-empty source (no fabricated device minimums)', () => {
    for (const entry of DEVICE_MATRIX) {
      expect(typeof entry.source).toBe('string');
      expect(entry.source.trim().length).toBeGreaterThan(0);
    }
  });

  it('ids are unique', () => {
    const ids = DEVICE_MATRIX.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("includes PRD #1418's starting regression cases: 390x844 and 1280x720", () => {
    const has390x844 = DEVICE_MATRIX.some((e) => e.width === 390 && e.height === 844);
    const has1280x720 = DEVICE_MATRIX.some((e) => e.width === 1280 && e.height === 720);
    expect(has390x844).toBe(true);
    expect(has1280x720).toBe(true);
  });

  it('includes the native split-pane floor from setup_accessibility.rs', () => {
    const pane = DEVICE_MATRIX.find((e) => e.kind === 'split-pane');
    expect(pane).toBeTruthy();
    expect(pane.width).toBe(320);
    expect(pane.height).toBe(320);
  });
});

describe('TEXT_SCALES', () => {
  it('is a non-empty ascending numeric list including 1 (100%) and 2 (200%)', () => {
    expect(Array.isArray(TEXT_SCALES)).toBe(true);
    expect(TEXT_SCALES.length).toBeGreaterThan(0);
    for (let i = 1; i < TEXT_SCALES.length; i++) {
      expect(TEXT_SCALES[i]).toBeGreaterThan(TEXT_SCALES[i - 1]);
    }
    expect(TEXT_SCALES[0]).toBe(1);
    expect(TEXT_SCALES[TEXT_SCALES.length - 1]).toBe(2);
  });
});

describe('BROWSER_ZOOMS', () => {
  it('is a non-empty ascending numeric list', () => {
    expect(Array.isArray(BROWSER_ZOOMS)).toBe(true);
    expect(BROWSER_ZOOMS.length).toBeGreaterThan(0);
    for (let i = 1; i < BROWSER_ZOOMS.length; i++) {
      expect(BROWSER_ZOOMS[i]).toBeGreaterThan(BROWSER_ZOOMS[i - 1]);
    }
  });
});

describe('DENSE_CONTENT', () => {
  const CATEGORIES = ['consoles', 'comms', 'gmLists'];

  it('carries all three surfaces PRD #1418 names', () => {
    for (const category of CATEGORIES) {
      expect(Array.isArray(DENSE_CONTENT[category])).toBe(true);
      expect(DENSE_CONTENT[category].length).toBeGreaterThan(0);
    }
  });

  it('every descriptor names a source and a positive repeatHint', () => {
    for (const category of CATEGORIES) {
      for (const descriptor of DENSE_CONTENT[category]) {
        expect(typeof descriptor.source).toBe('string');
        expect(descriptor.source.trim().length).toBeGreaterThan(0);
        expect(Number.isInteger(descriptor.repeatHint)).toBe(true);
        expect(descriptor.repeatHint).toBeGreaterThan(0);
      }
    }
  });

  it('every descriptor id is a REAL row in the String Table today', () => {
    for (const category of CATEGORIES) {
      for (const descriptor of DENSE_CONTENT[category]) {
        expect(has(descriptor.id)).toBe(true);
      }
    }
  });
});
