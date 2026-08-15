import { describe, it, expect } from 'vitest';
import { getRegionRenderSpec } from '../canvas-region.js';

// Tests for the pure getRegionRenderSpec(entity) function.
// The function maps a parsed region TOML entity to a rendering spec,
// covering shape geometry, fill colour at 15% alpha, and effect icon cluster.

describe('getRegionRenderSpec', () => {

  // ── Shape: sphere ────────────────────────────────────────────────────────

  describe('sphere region', () => {
    const sphereEntity = {
      tags: ['region', 'nebula'],
      transform: { position: [100.0, 0.0, -200.0] },
      shape: { type: 'sphere', radius: 150.0 },
      colour: [0.4, 0.6, 1.0],
      effects: {},
    };

    it('returns shape "circle" for sphere type', () => {
      const spec = getRegionRenderSpec(sphereEntity);
      expect(spec.shape).toBe('circle');
    });

    it('returns cx and cz from entity position', () => {
      const spec = getRegionRenderSpec(sphereEntity);
      expect(spec.cx).toBe(100.0);
      expect(spec.cz).toBe(-200.0);
    });

    it('returns radius from shape.radius', () => {
      const spec = getRegionRenderSpec(sphereEntity);
      expect(spec.radius).toBe(150.0);
    });

    it('defaults cx/cz to 0 when no position', () => {
      const entity = { ...sphereEntity };
      delete entity.transform;
      const spec = getRegionRenderSpec(entity);
      expect(spec.cx).toBe(0);
      expect(spec.cz).toBe(0);
    });
  });

  // ── Shape: box ───────────────────────────────────────────────────────────

  describe('box region', () => {
    const boxEntity = {
      tags: ['region', 'exclusion_zone'],
      transform: { position: [50.0, 0.0, 75.0] },
      shape: { type: 'box', half_extents: [80.0, 20.0, 40.0] },
      colour: [1.0, 0.3, 0.3],
      effects: { damage_zone: { damage_per_second: 5.0 } },
    };

    it('returns shape "rect" for box type', () => {
      const spec = getRegionRenderSpec(boxEntity);
      expect(spec.shape).toBe('rect');
    });

    it('returns cx and cz from entity position', () => {
      const spec = getRegionRenderSpec(boxEntity);
      expect(spec.cx).toBe(50.0);
      expect(spec.cz).toBe(75.0);
    });

    it('returns half_x from half_extents[0]', () => {
      const spec = getRegionRenderSpec(boxEntity);
      expect(spec.half_x).toBe(80.0);
    });

    it('returns half_z from half_extents[2]', () => {
      const spec = getRegionRenderSpec(boxEntity);
      expect(spec.half_z).toBe(40.0);
    });
  });

  // ── Shape: torus ─────────────────────────────────────────────────────────

  describe('torus region', () => {
    const torusEntity = {
      tags: ['region', 'asteroid_belt'],
      transform: { position: [0.0, 0.0, 0.0] },
      shape: { type: 'torus', inner_radius: 100.0, outer_radius: 250.0 },
      colour: [0.8, 0.6, 0.2],
      effects: { blocks_impulse: {} },
    };

    it('returns shape "torus" for torus type', () => {
      const spec = getRegionRenderSpec(torusEntity);
      expect(spec.shape).toBe('torus');
    });

    it('returns inner_radius from shape.inner_radius', () => {
      const spec = getRegionRenderSpec(torusEntity);
      expect(spec.inner_radius).toBe(100.0);
    });

    it('returns outer_radius from shape.outer_radius', () => {
      const spec = getRegionRenderSpec(torusEntity);
      expect(spec.outer_radius).toBe(250.0);
    });
  });

  // ── Colour and fill alpha ─────────────────────────────────────────────────

  describe('colour and fillAlpha', () => {
    it('passes through entity colour array', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 50.0 },
        colour: [0.3, 0.7, 0.9],
        effects: {},
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.colour).toEqual([0.3, 0.7, 0.9]);
    });

    it('fillAlpha is always 0.15', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 50.0 },
        colour: [1.0, 1.0, 1.0],
        effects: {},
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.fillAlpha).toBe(0.15);
    });

    it('uses default grey colour when entity has no colour', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 50.0 },
        effects: {},
      };
      const spec = getRegionRenderSpec(entity);
      expect(Array.isArray(spec.colour)).toBe(true);
      expect(spec.colour.length).toBe(3);
    });
  });

  // ── Effects list ──────────────────────────────────────────────────────────

  describe('active effects', () => {
    it('returns empty effects array when no effects present', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 50.0 },
        colour: [1.0, 1.0, 1.0],
        effects: {},
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toEqual([]);
    });

    it('returns ["damage_zone"] for a damage_zone entity', () => {
      const entity = {
        tags: ['region', 'damage_zone'],
        shape: { type: 'sphere', radius: 120.0 },
        colour: [1.0, 0.0, 0.0],
        effects: { damage_zone: { damage_per_second: 8.0 } },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('damage_zone');
      expect(spec.effects.length).toBe(1);
    });

    it('returns ["slow_zone"] for a slow_zone entity', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 80.0 },
        colour: [0.0, 0.5, 1.0],
        effects: { slow_zone: { thrust_modifier: 0.5 } },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('slow_zone');
    });

    it('returns ["blocks_impulse"] for asteroid belt region', () => {
      const entity = {
        tags: ['region', 'asteroid_belt'],
        shape: { type: 'torus', inner_radius: 100.0, outer_radius: 250.0 },
        colour: [0.8, 0.6, 0.2],
        effects: { blocks_impulse: {} },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('blocks_impulse');
    });

    it('returns ["radar_dampening"] for radar dampening region', () => {
      const entity = {
        tags: ['region', 'nebula'],
        shape: { type: 'sphere', radius: 220.0 },
        colour: [0.5, 0.2, 0.8],
        effects: { radar_dampening: { range_modifier: 0.4 } },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('radar_dampening');
    });

    it('returns ["comms_jammed"] for comms jammed region', () => {
      const entity = {
        tags: ['region', 'nebula'],
        shape: { type: 'sphere', radius: 150.0 },
        colour: [0.4, 0.1, 0.6],
        effects: { comms_jammed: {} },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('comms_jammed');
    });

    it('returns ["sensor_blind"] for sensor blind region', () => {
      const entity = {
        tags: ['region', 'nebula'],
        shape: { type: 'sphere', radius: 150.0 },
        colour: [0.4, 0.1, 0.6],
        effects: { sensor_blind: {} },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('sensor_blind');
    });

    it('returns all six effects when all are present', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 200.0 },
        colour: [0.5, 0.5, 0.5],
        effects: {
          damage_zone: { damage_per_second: 10.0 },
          slow_zone: { thrust_modifier: 0.5 },
          blocks_impulse: {},
          radar_dampening: { range_modifier: 0.3 },
          comms_jammed: {},
          sensor_blind: {},
        },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('damage_zone');
      expect(spec.effects).toContain('slow_zone');
      expect(spec.effects).toContain('blocks_impulse');
      expect(spec.effects).toContain('radar_dampening');
      expect(spec.effects).toContain('comms_jammed');
      expect(spec.effects).toContain('sensor_blind');
      expect(spec.effects.length).toBe(6);
    });

    it('returns multiple effects for kaleth_nebula (damage_zone + radar_dampening + sensor_blind)', () => {
      const entity = {
        tags: ['region', 'nebula'],
        shape: { type: 'sphere', radius: 220.0 },
        colour: [0.5, 0.2, 0.8],
        effects: {
          damage_zone: { damage_per_second: 3.0 },
          // Mirrors `assets/entities/region_kaleth_nebula.toml`. NEGATIVE:
          // `range_modifier` is a signed bonus on the radar-range slot, not a
          // multiplier, so -1.5 is the 0.4x the template wants. The render spec
          // only lists effect NAMES, so the value changes nothing here — it is
          // kept in step so the fixture does not teach the wrong sign.
          radar_dampening: { range_modifier: -1.5 },
          sensor_blind: {},
        },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toContain('damage_zone');
      expect(spec.effects).toContain('radar_dampening');
      expect(spec.effects).toContain('sensor_blind');
      expect(spec.effects.length).toBe(3);
    });

    it('handles missing effects property gracefully', () => {
      const entity = {
        tags: ['region'],
        shape: { type: 'sphere', radius: 50.0 },
        colour: [1.0, 1.0, 1.0],
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.effects).toEqual([]);
    });
  });

  // ── Effect icons ──────────────────────────────────────────────────────────

  describe('effectIcons map', () => {
    const anyEntity = {
      tags: ['region'],
      shape: { type: 'sphere', radius: 50.0 },
      colour: [1.0, 1.0, 1.0],
      effects: {},
    };

    it('effectIcons is always present', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(spec.effectIcons).toBeDefined();
      expect(typeof spec.effectIcons).toBe('object');
    });

    it('has an icon for damage_zone', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(typeof spec.effectIcons.damage_zone).toBe('string');
      expect(spec.effectIcons.damage_zone.length).toBeGreaterThan(0);
    });

    it('has an icon for slow_zone', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(typeof spec.effectIcons.slow_zone).toBe('string');
      expect(spec.effectIcons.slow_zone.length).toBeGreaterThan(0);
    });

    it('has an icon for blocks_impulse', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(typeof spec.effectIcons.blocks_impulse).toBe('string');
      expect(spec.effectIcons.blocks_impulse.length).toBeGreaterThan(0);
    });

    it('has an icon for radar_dampening', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(typeof spec.effectIcons.radar_dampening).toBe('string');
      expect(spec.effectIcons.radar_dampening.length).toBeGreaterThan(0);
    });

    it('has an icon for comms_jammed', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(typeof spec.effectIcons.comms_jammed).toBe('string');
      expect(spec.effectIcons.comms_jammed.length).toBeGreaterThan(0);
    });

    it('has an icon for sensor_blind', () => {
      const spec = getRegionRenderSpec(anyEntity);
      expect(typeof spec.effectIcons.sensor_blind).toBe('string');
      expect(spec.effectIcons.sensor_blind.length).toBeGreaterThan(0);
    });

    it('covers all six effect types', () => {
      const spec = getRegionRenderSpec(anyEntity);
      const expected = ['damage_zone', 'slow_zone', 'blocks_impulse', 'radar_dampening', 'comms_jammed', 'sensor_blind'];
      for (const key of expected) {
        expect(spec.effectIcons[key]).toBeDefined();
      }
    });
  });

  // ── Integration: real-world entity fixtures ───────────────────────────────

  describe('real-world entity fixtures', () => {
    it('region_asteroid_belt renders as torus with blocks_impulse', () => {
      const entity = {
        tags: ['region', 'asteroid_belt'],
        shape: { type: 'torus', inner_radius: 100.0, outer_radius: 250.0 },
        colour: [0.8, 0.6, 0.2],
        effects: { blocks_impulse: {} },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.shape).toBe('torus');
      expect(spec.inner_radius).toBe(100.0);
      expect(spec.outer_radius).toBe(250.0);
      expect(spec.effects).toEqual(['blocks_impulse']);
      expect(spec.fillAlpha).toBe(0.15);
    });

    it('region_radiation_zone renders as circle with damage_zone', () => {
      const entity = {
        tags: ['region', 'damage_zone', 'weapon_effect'],
        transform: { position: [300.0, 0.0, -150.0] },
        shape: { type: 'sphere', radius: 120.0 },
        colour: [1.0, 0.2, 0.0],
        effects: { damage_zone: { damage_per_second: 8.0 } },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.shape).toBe('circle');
      expect(spec.radius).toBe(120.0);
      expect(spec.cx).toBe(300.0);
      expect(spec.cz).toBe(-150.0);
      expect(spec.effects).toEqual(['damage_zone']);
    });

    it('region_nebula (comms+sensor_blind) renders as circle with two effects', () => {
      const entity = {
        tags: ['region', 'nebula'],
        shape: { type: 'sphere', radius: 150.0 },
        colour: [0.4, 0.3, 0.7],
        effects: { comms_jammed: {}, sensor_blind: {} },
      };
      const spec = getRegionRenderSpec(entity);
      expect(spec.shape).toBe('circle');
      expect(spec.effects).toContain('comms_jammed');
      expect(spec.effects).toContain('sensor_blind');
      expect(spec.effects.length).toBe(2);
    });
  });
});
