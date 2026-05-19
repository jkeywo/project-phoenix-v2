import { describe, it, expect } from 'vitest';
import { tagShape, RADAR_SHAPE } from '../tag-shape-map.js';

// Mirrors the runtime mapping in src/console/helm/client.rs:
//   ship  → Triangle
//   station → Square
//   (everything else) → Dot

describe('tag-shape-map', () => {
  describe('RADAR_SHAPE constants', () => {
    it('exports Triangle, Square, and Dot constants', () => {
      expect(RADAR_SHAPE.Triangle).toBe('Triangle');
      expect(RADAR_SHAPE.Square).toBe('Square');
      expect(RADAR_SHAPE.Dot).toBe('Dot');
    });
  });

  describe('tagShape — ship entities', () => {
    it('player_ship ["player","ship"] → Triangle', () => {
      expect(tagShape(['player', 'ship'])).toBe(RADAR_SHAPE.Triangle);
    });

    it('pirate_raider ["ship","npc","enemy"] → Triangle', () => {
      expect(tagShape(['ship', 'npc', 'enemy'])).toBe(RADAR_SHAPE.Triangle);
    });

    it('ship_harrow_patrol ["ship","npc","comms_contact"] → Triangle', () => {
      expect(tagShape(['ship', 'npc', 'comms_contact'])).toBe(RADAR_SHAPE.Triangle);
    });

    it('ship_harrow_warhawk ["ship","npc"] → Triangle', () => {
      expect(tagShape(['ship', 'npc'])).toBe(RADAR_SHAPE.Triangle);
    });

    it('ship_requiem_courier ["ship","npc","comms_contact"] → Triangle', () => {
      expect(tagShape(['ship', 'npc', 'comms_contact'])).toBe(RADAR_SHAPE.Triangle);
    });
  });

  describe('tagShape — station entities', () => {
    it('station_axiom ["station","comms_contact","allied"] → Square', () => {
      expect(tagShape(['station', 'comms_contact', 'allied'])).toBe(RADAR_SHAPE.Square);
    });

    it('station_outpost ["station","destructible"] → Square', () => {
      expect(tagShape(['station', 'destructible'])).toBe(RADAR_SHAPE.Square);
    });

    it('station_research_outpost ["station","comms_contact","science_facility"] → Square', () => {
      expect(tagShape(['station', 'comms_contact', 'science_facility'])).toBe(RADAR_SHAPE.Square);
    });
  });

  describe('tagShape — dot entities (asteroids, planets, stars, regions, fields)', () => {
    it('asteroid_large ["asteroid","gameplay","large"] → Dot', () => {
      expect(tagShape(['asteroid', 'gameplay', 'large'])).toBe(RADAR_SHAPE.Dot);
    });

    it('asteroid_small ["asteroid","gameplay","small"] → Dot', () => {
      expect(tagShape(['asteroid', 'gameplay', 'small'])).toBe(RADAR_SHAPE.Dot);
    });

    it('asteroid_cosmetic ["asteroid","cosmetic"] → Dot', () => {
      expect(tagShape(['asteroid', 'cosmetic'])).toBe(RADAR_SHAPE.Dot);
    });

    it('asteroid_field_main ["field","main","asteroid_field"] → Dot', () => {
      expect(tagShape(['field', 'main', 'asteroid_field'])).toBe(RADAR_SHAPE.Dot);
    });

    it('planet_earth ["planet","habitable"] → Dot', () => {
      expect(tagShape(['planet', 'habitable'])).toBe(RADAR_SHAPE.Dot);
    });

    it('star_sun ["star","center"] → Dot', () => {
      expect(tagShape(['star', 'center'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_nebula ["region","nebula"] → Dot', () => {
      expect(tagShape(['region', 'nebula'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_asteroid_belt ["region","asteroid_belt"] → Dot', () => {
      expect(tagShape(['region', 'asteroid_belt'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_kaleth_nebula ["region","nebula"] → Dot', () => {
      expect(tagShape(['region', 'nebula'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_radiation_zone ["region","damage_zone","weapon_effect"] → Dot', () => {
      expect(tagShape(['region', 'damage_zone', 'weapon_effect'])).toBe(RADAR_SHAPE.Dot);
    });
  });

  describe('tagShape — deterministic for multi-tag entities', () => {
    it('same tags in different order produce same result', () => {
      expect(tagShape(['npc', 'ship', 'enemy'])).toBe(tagShape(['ship', 'npc', 'enemy']));
    });

    it('same tags in different order produce same result for stations', () => {
      expect(tagShape(['comms_contact', 'station', 'allied'])).toBe(tagShape(['station', 'allied', 'comms_contact']));
    });
  });

  describe('tagShape — edge cases', () => {
    it('empty tags → Dot', () => {
      expect(tagShape([])).toBe(RADAR_SHAPE.Dot);
    });

    it('unknown tags → Dot', () => {
      expect(tagShape(['unknown', 'whatever'])).toBe(RADAR_SHAPE.Dot);
    });

    it('null/undefined tags → Dot', () => {
      expect(tagShape(null)).toBe(RADAR_SHAPE.Dot);
      expect(tagShape(undefined)).toBe(RADAR_SHAPE.Dot);
    });
  });
});
