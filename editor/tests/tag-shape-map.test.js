import { describe, it, expect } from 'vitest';
import { iconShape, RADAR_SHAPE } from '../tag-shape-map.js';

describe('tag-shape-map', () => {
  describe('RADAR_SHAPE constants', () => {
    it('exports Triangle, Square, Diamond, Ring, and Dot constants', () => {
      expect(RADAR_SHAPE.Triangle).toBe('Triangle');
      expect(RADAR_SHAPE.Square).toBe('Square');
      expect(RADAR_SHAPE.Diamond).toBe('Diamond');
      expect(RADAR_SHAPE.Ring).toBe('Ring');
      expect(RADAR_SHAPE.Dot).toBe('Dot');
    });
  });

  describe('iconShape — ship-like icons', () => {
    it('"ship" → Triangle', () => {
      expect(iconShape('ship')).toBe(RADAR_SHAPE.Triangle);
    });

    it('"playerShip" → Triangle', () => {
      expect(iconShape('playerShip')).toBe(RADAR_SHAPE.Triangle);
    });

    it('"destroyer" → Triangle', () => {
      expect(iconShape('destroyer')).toBe(RADAR_SHAPE.Triangle);
    });

    it('"cruiser" → Triangle', () => {
      expect(iconShape('cruiser')).toBe(RADAR_SHAPE.Triangle);
    });

    it('"battleship" → Triangle', () => {
      expect(iconShape('battleship')).toBe(RADAR_SHAPE.Triangle);
    });
  });

  describe('iconShape — station icon', () => {
    it('"station" → Diamond', () => {
      expect(iconShape('station')).toBe(RADAR_SHAPE.Diamond);
    });
  });

  describe('iconShape — planet icon', () => {
    it('"planet" → Ring', () => {
      expect(iconShape('planet')).toBe(RADAR_SHAPE.Ring);
    });
  });

  describe('iconShape — dot icons (asteroid, star, torpedo, unrecognised)', () => {
    it('"asteroid" → Dot', () => {
      expect(iconShape('asteroid')).toBe(RADAR_SHAPE.Dot);
    });

    it('"star" → Dot', () => {
      expect(iconShape('star')).toBe(RADAR_SHAPE.Dot);
    });

    it('"torpedo" → Dot', () => {
      expect(iconShape('torpedo')).toBe(RADAR_SHAPE.Dot);
    });

    it('case-insensitive: "STATION" → Diamond', () => {
      expect(iconShape('STATION')).toBe(RADAR_SHAPE.Diamond);
    });
  });

  describe('iconShape — edge cases', () => {
    it('empty string → Dot', () => {
      expect(iconShape('')).toBe(RADAR_SHAPE.Dot);
    });

    it('unrecognised icon name → Dot (no whitelist, just no glyph match)', () => {
      expect(iconShape('frigate')).toBe(RADAR_SHAPE.Dot);
    });

    it('null/undefined → Dot', () => {
      expect(iconShape(null)).toBe(RADAR_SHAPE.Dot);
      expect(iconShape(undefined)).toBe(RADAR_SHAPE.Dot);
    });
  });
});
