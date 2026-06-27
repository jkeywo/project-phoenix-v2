import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { parseEntityToml, stringifyEntityToml, validateEntityToml } from '../entity-toml.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

function readEntity(name) {
  return readFileSync(resolve(projectRoot, 'assets/entities', name), 'utf-8');
}

describe('entity-toml', () => {
  describe('parseEntityToml', () => {
    it('parses a basic entity TOML', () => {
      const toml = 'tags = ["ship", "npc"]\n\n[hull]\nhull_integrity = 100.0\n';
      const result = parseEntityToml(toml);
      expect(result.tags).toEqual(['ship', 'npc']);
      expect(result.hull.hull_integrity).toBe(100.0);
    });

    it('parses pirate_raider.toml', () => {
      const text = readEntity('pirate_raider.toml');
      const result = parseEntityToml(text);
      expect(result.tags).toContain('ship');
      expect(result.tags).toContain('npc');
      expect(result.faction).toBeTruthy();
      expect(result.hull.hull_integrity).toBe(30.0);
      expect(result.collider.shape).toBe('Capsule');
      expect(Array.isArray(result.behaviour.doctrine)).toBe(true);
      expect(result.behaviour.doctrine.length).toBeGreaterThanOrEqual(1);
      expect(result.behaviour.doctrine[0].id).toBe('patrol-sector');
    });

    it('parses asteroid_common_1_large.toml', () => {
      const text = readEntity('asteroid_common_1_large.toml');
      const result = parseEntityToml(text);
      expect(result.tags).toContain('asteroid');
    });

    it('throws on invalid TOML', () => {
      expect(() => parseEntityToml('not valid')).toThrow();
    });
  });

  describe('stringifyEntityToml', () => {
    it('serializes a parsed entity back to string', () => {
      const obj = { tags: ['ship'], hull: { hull_integrity: 100.0 } };
      const result = stringifyEntityToml(obj);
      expect(typeof result).toBe('string');
      expect(result).toContain('tags');
    });

    it('produces parseable TOML', () => {
      const obj = { tags: ['test'], hull: { hull_integrity: 50.0 } };
      const serialized = stringifyEntityToml(obj);
      const reparsed = parseEntityToml(serialized);
      expect(reparsed.tags).toEqual(['test']);
      expect(reparsed.hull.hull_integrity).toBe(50.0);
    });
  });

  describe('validateEntityToml', () => {
    it('returns valid for entity with tags', () => {
      const result = validateEntityToml({ tags: ['ship'] });
      expect(result.valid).toBe(true);
    });

    it('returns invalid for entity without tags', () => {
      const result = validateEntityToml({ hull: {} });
      expect(result.valid).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('returns invalid for non-object', () => {
      const result = validateEntityToml('string');
      expect(result.valid).toBe(false);
    });
  });

  describe('round-trip shipped entities', () => {
    const entityFiles = [
      'pirate_raider.toml',
      'asteroid_common_1_large.toml',
      'asteroid_common_1_small.toml',
      'asteroid_common_1_cosmetic.toml',
      'player_ship.toml',
      'station_axiom.toml',
      'station_outpost.toml',
      'star_sun.toml',
      'planet_earth.toml',
    ];

    for (const file of entityFiles) {
      it(`${file} survives parse → stringify → parse`, () => {
        const originalText = readEntity(file);
        const parsed = parseEntityToml(originalText);
        const serialized = stringifyEntityToml(parsed);
        const reparsed = parseEntityToml(serialized);

        expect(reparsed.tags).toEqual(parsed.tags);
        if (parsed.hull) {
          expect(reparsed.hull).toEqual(parsed.hull);
        }
        if (parsed.collider) {
          expect(reparsed.collider).toEqual(parsed.collider);
        }
      });
    }
  });
});
