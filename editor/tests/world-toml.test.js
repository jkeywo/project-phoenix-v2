import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';
import { parseWorldToml, stringifyWorldToml, validateWorldToml } from '../world-toml.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../..');

function readWorld(name) {
  return readFileSync(resolve(projectRoot, 'assets/worlds', name), 'utf-8');
}

describe('world-toml', () => {
  describe('parseWorldToml', () => {
    it('parses a basic world TOML string', () => {
      const toml = '[global]\nseed = 42\n\n[anchors]\ntest = [1.0, 0.0, 2.0]\n';
      const result = parseWorldToml(toml);
      expect(result.global.seed).toBe(42);
      expect(result.anchors.test).toEqual([1.0, 0.0, 2.0]);
    });

    it('parses default.toml', () => {
      const text = readWorld('default.toml');
      const result = parseWorldToml(text);
      expect(result.global.seed).toBe(42);
      expect(result.anchors.starbase_alpha).toEqual([500.0, 0.0, 0.0]);
      expect(Array.isArray(result.entity)).toBe(true);
      expect(result.entity.length).toBeGreaterThanOrEqual(5);
      expect(Array.isArray(result.trigger)).toBe(true);
      expect(Array.isArray(result.comms)).toBe(true);
    });

    it('parses patrol.toml', () => {
      const text = readWorld('patrol.toml');
      const result = parseWorldToml(text);
      expect(result.global.seed).toBe(42);
      expect(result.anchors.patrol_alpha).toEqual([300.0, 0.0, -300.0]);
      expect(result.entity.length).toBe(4);
    });

    it('throws on invalid TOML', () => {
      expect(() => parseWorldToml('not valid toml ===')).toThrow();
    });
  });

  describe('stringifyWorldToml', () => {
    it('serializes a parsed world back to string', () => {
      const obj = {
        global: { seed: 42 },
        anchors: { test: [1.0, 0.0, 2.0] },
      };
      const result = stringifyWorldToml(obj);
      expect(result).toBeTruthy();
      expect(typeof result).toBe('string');
    });

    it('produces parseable TOML', () => {
      const obj = {
        global: { seed: 99 },
        anchors: { a: [0.0, 0.0, 0.0] },
      };
      const result = stringifyWorldToml(obj);
      const reparsed = parseWorldToml(result);
      expect(reparsed.global.seed).toBe(99);
      expect(reparsed.anchors.a).toEqual([0.0, 0.0, 0.0]);
    });
  });

  describe('validateWorldToml', () => {
    it('returns valid for a correct world object', () => {
      const obj = {
        global: { seed: 42 },
        anchors: { a: [0.0, 0.0, 0.0] },
      };
      const result = validateWorldToml(obj);
      expect(result.valid).toBe(true);
    });

    it('returns invalid for missing global', () => {
      const result = validateWorldToml({ anchors: {} });
      expect(result.valid).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('returns invalid for non-object', () => {
      const result = validateWorldToml(null);
      expect(result.valid).toBe(false);
    });
  });

  describe('round-trip shipped worlds', () => {
    it('default.toml survives parse → stringify → parse with same structure', () => {
      const originalText = readWorld('default.toml');
      const parsed = parseWorldToml(originalText);
      const serialized = stringifyWorldToml(parsed);
      const reparsed = parseWorldToml(serialized);

      expect(reparsed.global.seed).toEqual(parsed.global.seed);
      expect(reparsed.anchors).toEqual(parsed.anchors);
      expect(reparsed.entity.length).toEqual(parsed.entity.length);
      expect(reparsed.trigger.length).toEqual(parsed.trigger.length);
      expect(reparsed.comms.length).toEqual(parsed.comms.length);

      for (let i = 0; i < parsed.entity.length; i++) {
        expect(reparsed.entity[i].template_path).toBe(parsed.entity[i].template_path);
      }
    });

    it('patrol.toml survives parse → stringify → parse with same structure', () => {
      const originalText = readWorld('patrol.toml');
      const parsed = parseWorldToml(originalText);
      const serialized = stringifyWorldToml(parsed);
      const reparsed = parseWorldToml(serialized);

      expect(reparsed.global.seed).toEqual(parsed.global.seed);
      expect(reparsed.anchors).toEqual(parsed.anchors);
      expect(reparsed.entity.length).toEqual(parsed.entity.length);
    });
  });
});
