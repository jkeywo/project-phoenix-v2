import { describe, it, expect } from 'vitest';
import { tagShape, RADAR_SHAPE } from '../tag-shape-map.js';

// Mirrors the runtime table `tags_to_radar_layer` + `layer_to_icon` in
// src/gui/radar.rs. The full editor mapping is:
//   ship | pirate          → Triangle
//   asteroid | asteroid_field → Dot
//   station               → Diamond
//   missile | torpedo     → Dot
//   planet                → Ring
//   star                  → Dot
//   region                → Dot   (regions are filtered from runtime radar
//                                  entirely; editor uses Dot as a generic)
//   (everything else)     → Dot

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

  describe('tagShape — ship entities', () => {
    it('player_ship ["player","ship"] → Triangle', () => {
      expect(tagShape(['player', 'ship'])).toBe(RADAR_SHAPE.Triangle);
    });

    it('pirate-tagged entity ["pirate","npc","enemy"] → Triangle', () => {
      expect(tagShape(['pirate', 'npc', 'enemy'])).toBe(RADAR_SHAPE.Triangle);
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
    it('station_axiom ["station","comms_contact","allied"] → Diamond', () => {
      expect(tagShape(['station', 'comms_contact', 'allied'])).toBe(RADAR_SHAPE.Diamond);
    });

    it('station_outpost ["station","destructible"] → Diamond', () => {
      expect(tagShape(['station', 'destructible'])).toBe(RADAR_SHAPE.Diamond);
    });

    it('station_research_outpost ["station","comms_contact","science_facility"] → Diamond', () => {
      expect(tagShape(['station', 'comms_contact', 'science_facility'])).toBe(RADAR_SHAPE.Diamond);
    });
  });

  describe('tagShape — planet entities', () => {
    it('planet_earth ["planet","habitable"] → Ring', () => {
      expect(tagShape(['planet', 'habitable'])).toBe(RADAR_SHAPE.Ring);
    });

    it('planet_mars ["planet","barren"] → Ring', () => {
      expect(tagShape(['planet', 'barren'])).toBe(RADAR_SHAPE.Ring);
    });
  });

  describe('tagShape — dot entities (asteroids, stars, missiles, regions, fields)', () => {
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

    it('star_sun ["star","center"] → Dot', () => {
      expect(tagShape(['star', 'center'])).toBe(RADAR_SHAPE.Dot);
    });

    it('missile-tagged entity → Dot', () => {
      expect(tagShape(['missile'])).toBe(RADAR_SHAPE.Dot);
    });

    it('torpedo-tagged entity → Dot', () => {
      expect(tagShape(['torpedo'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_nebula ["region","nebula"] → Dot', () => {
      expect(tagShape(['region', 'nebula'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_asteroid_belt ["region","asteroid_belt"] → Dot', () => {
      expect(tagShape(['region', 'asteroid_belt'])).toBe(RADAR_SHAPE.Dot);
    });

    it('region_radiation_zone ["region","damage_zone","weapon_effect"] → Dot', () => {
      expect(tagShape(['region', 'damage_zone', 'weapon_effect'])).toBe(RADAR_SHAPE.Dot);
    });
  });

  describe('tagShape — precedence', () => {
    it('region tag forces Dot even when combined with station', () => {
      // Matches the Rust precedence: region check is first.
      expect(tagShape(['region', 'station'])).toBe(RADAR_SHAPE.Dot);
    });

    it('ship beats station when both tags present', () => {
      // Matches the Rust precedence: ship is checked before station.
      expect(tagShape(['station', 'ship'])).toBe(RADAR_SHAPE.Triangle);
    });

    it('station beats planet when both tags present', () => {
      expect(tagShape(['planet', 'station'])).toBe(RADAR_SHAPE.Diamond);
    });

    it('same tags in different order produce same result', () => {
      expect(tagShape(['npc', 'ship', 'enemy'])).toBe(tagShape(['ship', 'npc', 'enemy']));
      expect(tagShape(['comms_contact', 'station', 'allied']))
        .toBe(tagShape(['station', 'allied', 'comms_contact']));
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
