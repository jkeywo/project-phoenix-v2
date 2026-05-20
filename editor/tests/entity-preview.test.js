import { describe, it, expect } from 'vitest';
import { computeEntityPreview } from '../entity-preview.js';

// Test fixtures (plain objects matching smol-toml parse output)

const shipWithRadar = {
  tags: ['ship', 'player'],
  radar_appearance: { colour: [0.6, 0.8, 1.0], radius: 6.0 },
};

const stationWithRadar = {
  tags: ['station', 'allied'],
  radar_appearance: { colour: [0.3, 0.8, 0.6], radius: 18.0 },
};

const asteroidNoRadar = {
  tags: ['asteroid', 'gameplay', 'large'],
};

const entityWithSphereShape = {
  tags: ['region', 'nebula'],
  shape: { type: 'sphere', radius: 150.0 },
};

const entityWithBoxShape = {
  tags: ['region', 'hazard'],
  shape: { type: 'box', half_extents: [100.0, 50.0, 200.0], yaw: 0.5 },
};

const entityWithTorusShape = {
  tags: ['region', 'asteroid_belt'],
  shape: { type: 'torus', inner_radius: 100.0, outer_radius: 250.0 },
};

const entityWithAsteroidField = {
  tags: ['field', 'main', 'asteroid_field'],
  asteroid_field: { inner_radius: 100.0, outer_radius: 200.0, density: 0.005 },
};

const entityWithConsoles = {
  tags: ['ship', 'player'],
  helm_console: { max_speed: 50.0 },
  weapons_console: { beam_range: 40.0 },
  captain_console: {},
};

const entityWithHull = {
  tags: ['ship'],
  hull: { hull_integrity: 200.0 },
};

const entityWithFaction = {
  tags: ['ship', 'npc'],
  faction: 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb',
};

const minimalEntity = {
  tags: ['test'],
};

describe('computeEntityPreview', () => {
  describe('radar_appearance', () => {
    it('entity with radar_appearance returns correct radarShape, radarColour, radarRadius', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.radarShape).toBe('Triangle');
      expect(result.radarColour).toEqual([0.6, 0.8, 1.0]);
      expect(result.radarRadius).toBe(6.0);
    });

    it('entity without radar_appearance returns shape X, colour null, radius null', () => {
      const result = computeEntityPreview(asteroidNoRadar);
      expect(result.radarShape).toBe('X');
      expect(result.radarColour).toBeNull();
      expect(result.radarRadius).toBeNull();
    });

    it('ship-tagged entity with radar_appearance returns Triangle shape', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.radarShape).toBe('Triangle');
    });

    it('station-tagged entity with radar_appearance returns Square shape', () => {
      const result = computeEntityPreview(stationWithRadar);
      expect(result.radarShape).toBe('Square');
    });
  });

  describe('collider', () => {
    it('entity with collider returns collider data', () => {
      const entity = {
        tags: ['ship'],
        collider: { shape: 'Capsule', radius: 3.0, length: 6.0 },
      };
      const result = computeEntityPreview(entity);
      expect(result.colliderShape).toBe('Capsule');
      expect(result.colliderRadius).toBe(3.0);
      expect(result.colliderLength).toBe(6.0);
    });

    it('entity without collider returns null shape and zero dimensions', () => {
      const result = computeEntityPreview(minimalEntity);
      expect(result.colliderShape).toBeNull();
      expect(result.colliderRadius).toBe(0);
      expect(result.colliderLength).toBe(0);
    });
  });

  describe('regionShape', () => {
    it('sphere shape derives regionShape correctly', () => {
      const result = computeEntityPreview(entityWithSphereShape);
      expect(result.regionShape).toEqual({ type: 'sphere', radius: 150.0 });
    });

    it('box shape derives regionShape with halfExtents and yaw', () => {
      const result = computeEntityPreview(entityWithBoxShape);
      expect(result.regionShape).toEqual({
        type: 'box',
        halfExtents: [100.0, 50.0, 200.0],
        yaw: 0.5,
      });
    });

    it('torus shape derives regionShape with innerRadius and outerRadius', () => {
      const result = computeEntityPreview(entityWithTorusShape);
      expect(result.regionShape).toEqual({
        type: 'torus',
        innerRadius: 100.0,
        outerRadius: 250.0,
      });
    });

    it('entity without shape section returns null regionShape', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.regionShape).toBeNull();
    });
  });

  describe('asteroidField', () => {
    it('entity with asteroid_field returns innerRadius and outerRadius', () => {
      const result = computeEntityPreview(entityWithAsteroidField);
      expect(result.asteroidField).toEqual({ innerRadius: 100.0, outerRadius: 200.0 });
    });

    it('entity without asteroid_field returns null', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.asteroidField).toBeNull();
    });
  });

  describe('textOverlay.consoles', () => {
    it('entity with console sections lists their keys', () => {
      const result = computeEntityPreview(entityWithConsoles);
      expect(result.textOverlay.consoles).toContain('helm_console');
      expect(result.textOverlay.consoles).toContain('weapons_console');
      expect(result.textOverlay.consoles).toContain('captain_console');
      expect(result.textOverlay.consoles).toHaveLength(3);
    });

    it('entity without console sections returns empty array', () => {
      const result = computeEntityPreview(asteroidNoRadar);
      expect(result.textOverlay.consoles).toEqual([]);
    });
  });

  describe('textOverlay.hullTotal', () => {
    it('entity with hull.hull_integrity returns its value', () => {
      const result = computeEntityPreview(entityWithHull);
      expect(result.textOverlay.hullTotal).toBe(200.0);
    });

    it('entity without hull_integrity returns null', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.textOverlay.hullTotal).toBeNull();
    });
  });

  describe('textOverlay.faction', () => {
    it('entity with faction resolves via provided map', () => {
      const factionMap = new Map([
        ['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb', 'Pirate'],
      ]);
      const result = computeEntityPreview(entityWithFaction, factionMap);
      expect(result.textOverlay.faction).toBe('Pirate');
    });

    it('entity with faction not in map returns UUID', () => {
      const result = computeEntityPreview(entityWithFaction);
      expect(result.textOverlay.faction).toBe('bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb');
    });

    it('entity without faction returns null', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.textOverlay.faction).toBeNull();
    });
  });

  describe('showForwardArrow', () => {
    it('is always true', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.showForwardArrow).toBe(true);
    });
  });

  describe('textOverlay.tags', () => {
    it('includes entity tags', () => {
      const result = computeEntityPreview(shipWithRadar);
      expect(result.textOverlay.tags).toEqual(['ship', 'player']);
    });

    it('returns empty array for entity with no tags key', () => {
      const result = computeEntityPreview({});
      expect(result.textOverlay.tags).toEqual([]);
    });
  });

  describe('edge cases', () => {
    it('null entity returns null', () => {
      expect(computeEntityPreview(null)).toBeNull();
    });

    it('undefined entity returns null', () => {
      expect(computeEntityPreview(undefined)).toBeNull();
    });

    it('entity with no special sections returns defaults gracefully', () => {
      const result = computeEntityPreview(minimalEntity);
      expect(result.colliderShape).toBeNull();
      expect(result.colliderRadius).toBe(0);
      expect(result.colliderLength).toBe(0);
      expect(result.radarShape).toBe('X');
      expect(result.radarColour).toBeNull();
      expect(result.radarRadius).toBeNull();
      expect(result.regionShape).toBeNull();
      expect(result.asteroidField).toBeNull();
      expect(result.showForwardArrow).toBe(true);
      expect(result.textOverlay.tags).toEqual(['test']);
      expect(result.textOverlay.faction).toBeNull();
      expect(result.textOverlay.consoles).toEqual([]);
      expect(result.textOverlay.hullTotal).toBeNull();
    });

    it('entity with collider shape set to Ball', () => {
      const entity = {
        tags: ['station'],
        collider: { shape: 'Ball', radius: 12.0, length: 0.0 },
      };
      const result = computeEntityPreview(entity);
      expect(result.colliderShape).toBe('Ball');
      expect(result.colliderRadius).toBe(12.0);
    });
  });
});
