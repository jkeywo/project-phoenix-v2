import { describe, it, expect } from 'vitest';
import {
  orientationFor,
  updateOrientation,
  currentOrientation,
} from '../../gui/device-orientation.js';

// ── orientationFor (pure classifier, mirrors framing.rs detect_orientation) ──

describe('orientationFor', () => {
  it('classifies wide as landscape', () => {
    expect(orientationFor(800, 400)).toBe('landscape');
  });

  it('classifies tall as portrait', () => {
    expect(orientationFor(400, 800)).toBe('portrait');
  });

  it('treats an exactly-square aspect as landscape (>= 1.0 threshold)', () => {
    // Mirrors the Rust `aspect >= 1.0` threshold (square counts as landscape).
    expect(orientationFor(500, 500)).toBe('landscape');
  });

  it('classifies a barely-wide aspect as landscape', () => {
    expect(orientationFor(501, 500)).toBe('landscape');
  });

  it('classifies a barely-tall aspect as portrait', () => {
    expect(orientationFor(499, 500)).toBe('portrait');
  });

  it('falls back to portrait for a zero/invalid height', () => {
    expect(orientationFor(800, 0)).toBe('portrait');
    expect(orientationFor(800, -10)).toBe('portrait');
  });
});

// ── updateOrientation + singleton (resize-driven update) ─────────────────────

describe('updateOrientation singleton', () => {
  it('updates the singleton from an explicit window-like object', () => {
    updateOrientation({ innerWidth: 1000, innerHeight: 500 });
    expect(currentOrientation()).toBe('landscape');

    updateOrientation({ innerWidth: 500, innerHeight: 1000 });
    expect(currentOrientation()).toBe('portrait');
  });

  it('returns the freshly-computed value', () => {
    expect(updateOrientation({ innerWidth: 1200, innerHeight: 400 })).toBe('landscape');
    expect(updateOrientation({ innerWidth: 300, innerHeight: 900 })).toBe('portrait');
  });

  it('reflects a simulated resize: portrait -> landscape', () => {
    updateOrientation({ innerWidth: 360, innerHeight: 740 });
    expect(currentOrientation()).toBe('portrait');
    // Phone rotates to landscape.
    updateOrientation({ innerWidth: 740, innerHeight: 360 });
    expect(currentOrientation()).toBe('landscape');
  });
});
