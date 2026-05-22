import { describe, it, expect } from 'vitest';
import { resolveEntityAppearance, RADAR_SHAPE_FALLBACK } from '../canvas-world.js';

// Tests for the pure logic that drives canvas rendering of world-mode entities.
// resolveEntityAppearance(entity) returns { colour, radius, shape, hasFallback }
//   - colour: [r, g, b] normalised 0-1 floats (from radar_appearance.colour)
//   - radius: positive number (from radar_appearance.radius)
//   - shape:  RadarShape string (from tag-shape-map)
//   - hasFallback: true if radar_appearance was absent (render as X)

describe('resolveEntityAppearance', () => {
  describe('entities with [radar_appearance]', () => {
    it('pirate_raider returns red colour, radius 4, Triangle shape', () => {
      const entity = {
        tags: ['ship', 'npc', 'enemy'],
        radar_appearance: { colour: [1.0, 0.2, 0.2], radius: 4.0 },
      };
      const result = resolveEntityAppearance(entity);
      expect(result.colour).toEqual([1.0, 0.2, 0.2]);
      expect(result.radius).toBe(4.0);
      expect(result.shape).toBe('Triangle');
      expect(result.hasFallback).toBe(false);
    });

    it('station_axiom returns green colour, radius 18, Diamond shape', () => {
      const entity = {
        tags: ['station', 'comms_contact', 'allied'],
        radar_appearance: { colour: [0.3, 0.8, 0.6], radius: 18.0 },
      };
      const result = resolveEntityAppearance(entity);
      expect(result.colour).toEqual([0.3, 0.8, 0.6]);
      expect(result.radius).toBe(18.0);
      expect(result.shape).toBe('Diamond');
      expect(result.hasFallback).toBe(false);
    });

    it('star_sun returns yellow colour, radius 50, Dot shape', () => {
      const entity = {
        tags: ['star', 'center'],
        radar_appearance: { colour: [1.0, 0.85, 0.3], radius: 50.0 },
      };
      const result = resolveEntityAppearance(entity);
      expect(result.colour).toEqual([1.0, 0.85, 0.3]);
      expect(result.radius).toBe(50.0);
      expect(result.shape).toBe('Dot');
      expect(result.hasFallback).toBe(false);
    });

    it('planet_earth returns blue colour, radius 20, Ring shape', () => {
      const entity = {
        tags: ['planet', 'habitable'],
        radar_appearance: { colour: [0.0, 0.6, 1.0], radius: 20.0 },
      };
      const result = resolveEntityAppearance(entity);
      expect(result.colour).toEqual([0.0, 0.6, 1.0]);
      expect(result.radius).toBe(20.0);
      expect(result.shape).toBe('Ring');
      expect(result.hasFallback).toBe(false);
    });

    it('player_ship returns light-blue colour, radius 6, Triangle shape', () => {
      const entity = {
        tags: ['player', 'ship'],
        radar_appearance: { colour: [0.6, 0.8, 1.0], radius: 6.0 },
      };
      const result = resolveEntityAppearance(entity);
      expect(result.colour).toEqual([0.6, 0.8, 1.0]);
      expect(result.radius).toBe(6.0);
      expect(result.shape).toBe('Triangle');
      expect(result.hasFallback).toBe(false);
    });
  });

  describe('entities WITHOUT [radar_appearance] — X fallback', () => {
    it('asteroid_large (no radar_appearance) → hasFallback true', () => {
      const entity = {
        tags: ['asteroid', 'gameplay', 'large'],
        // no radar_appearance
      };
      const result = resolveEntityAppearance(entity);
      expect(result.hasFallback).toBe(true);
    });

    it('asteroid_small (no radar_appearance) → hasFallback true', () => {
      const entity = {
        tags: ['asteroid', 'gameplay', 'small'],
      };
      const result = resolveEntityAppearance(entity);
      expect(result.hasFallback).toBe(true);
    });

    it('asteroid_cosmetic (no radar_appearance) → hasFallback true', () => {
      const entity = {
        tags: ['asteroid', 'cosmetic'],
      };
      const result = resolveEntityAppearance(entity);
      expect(result.hasFallback).toBe(true);
    });

    it('region_nebula (no radar_appearance) → hasFallback true', () => {
      const entity = {
        tags: ['region', 'nebula'],
      };
      const result = resolveEntityAppearance(entity);
      expect(result.hasFallback).toBe(true);
    });

    it('asteroid_field_main (no radar_appearance) → hasFallback true', () => {
      const entity = {
        tags: ['field', 'main', 'asteroid_field'],
      };
      const result = resolveEntityAppearance(entity);
      expect(result.hasFallback).toBe(true);
    });

    it('fallback entity still gets Dot shape from tags', () => {
      const entity = { tags: ['asteroid', 'gameplay', 'large'] };
      const result = resolveEntityAppearance(entity);
      expect(result.shape).toBe('Dot');
      expect(result.hasFallback).toBe(true);
    });

    it('fallback entity with ship tag still gets Triangle shape', () => {
      // A ship entity missing radar_appearance should still get Triangle
      const entity = { tags: ['ship', 'npc'] };
      const result = resolveEntityAppearance(entity);
      expect(result.shape).toBe('Triangle');
      expect(result.hasFallback).toBe(true);
    });

    it('fallback neutral colour is provided', () => {
      const entity = { tags: ['asteroid'] };
      const result = resolveEntityAppearance(entity);
      // Should provide a neutral fallback colour (not undefined/null)
      expect(result.colour).toBeDefined();
      expect(Array.isArray(result.colour)).toBe(true);
      expect(result.colour.length).toBe(3);
    });

    it('fallback radius is a positive number', () => {
      const entity = { tags: ['asteroid'] };
      const result = resolveEntityAppearance(entity);
      expect(typeof result.radius).toBe('number');
      expect(result.radius).toBeGreaterThan(0);
    });
  });

  describe('RADAR_SHAPE_FALLBACK constant', () => {
    it('exports a fallback marker string', () => {
      expect(typeof RADAR_SHAPE_FALLBACK).toBe('string');
      expect(RADAR_SHAPE_FALLBACK.length).toBeGreaterThan(0);
    });
  });

  describe('anchor-resolved position', () => {
    it('entity with inline position returns that position', () => {
      const entity = {
        tags: ['ship'],
        radar_appearance: { colour: [1.0, 0.0, 0.0], radius: 4.0 },
        transform: { position: [100.0, 0.0, -200.0] },
      };
      const result = resolveEntityAppearance(entity, {});
      expect(result.x).toBe(100.0);
      expect(result.z).toBe(-200.0);
    });

    it('entity with anchor resolves position from flat anchor map', () => {
      const anchors = {
        patrol_alpha: [300.0, 0.0, -300.0],
      };
      const entity = {
        tags: ['ship', 'npc'],
        radar_appearance: { colour: [1.0, 0.2, 0.2], radius: 4.0 },
        transform: { anchor: 'patrol_alpha' },
      };
      const result = resolveEntityAppearance(entity, anchors);
      expect(result.x).toBe(300.0);
      expect(result.z).toBe(-300.0);
    });

    it('entity with no position or anchor defaults to [0, 0]', () => {
      const entity = { tags: ['asteroid'], radar_appearance: { colour: [0.8, 0.7, 0.4], radius: 5.0 } };
      const result = resolveEntityAppearance(entity, {});
      expect(result.x).toBe(0);
      expect(result.z).toBe(0);
    });
  });
});
