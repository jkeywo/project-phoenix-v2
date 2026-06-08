import { describe, it, expect } from 'vitest';
import { bezelSrc, BEZEL_SLOTS } from '../../gui/phone-bezel.js';

describe('bezelSrc', () => {
  it('exposes all eight slot identifiers', () => {
    expect(BEZEL_SLOTS).toEqual([
      'corner-tl',
      'corner-tr',
      'corner-bl',
      'corner-br',
      'edge-top',
      'edge-bottom',
      'edge-left',
      'edge-right',
    ]);
  });

  for (const slot of [
    'corner-tl',
    'corner-tr',
    'corner-bl',
    'corner-br',
    'edge-top',
    'edge-bottom',
    'edge-left',
    'edge-right',
  ]) {
    it(`returns the normal URL for ${slot} when alert is false`, () => {
      expect(bezelSrc(slot, false)).toBe(`gui/borders/phone-${slot}.png`);
    });

    it(`returns the -alert URL for ${slot} when alert is true`, () => {
      expect(bezelSrc(slot, true)).toBe(`gui/borders/phone-${slot}-alert.png`);
    });
  }

  it('treats truthy non-boolean values as alert=true', () => {
    expect(bezelSrc('corner-tl', 1)).toBe('gui/borders/phone-corner-tl-alert.png');
    expect(bezelSrc('corner-tl', 'yes')).toBe('gui/borders/phone-corner-tl-alert.png');
  });

  it('treats falsy non-boolean values as alert=false', () => {
    expect(bezelSrc('corner-tl', 0)).toBe('gui/borders/phone-corner-tl.png');
    expect(bezelSrc('corner-tl', null)).toBe('gui/borders/phone-corner-tl.png');
    expect(bezelSrc('corner-tl', undefined)).toBe('gui/borders/phone-corner-tl.png');
  });
});
